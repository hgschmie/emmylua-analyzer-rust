//! Narrow reads through a stable local key after a definite write.
//!
//! Computed keys do not have a static member ID. Track the declarations of the
//! table and key instead. Stop at conflicting effects and uncertain joins;
//! preserve facts across opaque calls and loops without visible conflicting writes.

// Bound backward assignment search; exhaustion leaves presence unproven.
const MAX_ASSIGNMENT_TRACE_STEPS: usize = 128;

use emmylua_parser::{LuaAstNode, LuaExpr, LuaIndexExpr, LuaStat, LuaVarExpr};
use rowan::TextSize;

use super::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaType, SemanticModel, computed_effects};

pub(super) fn assigned_type(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    offset: TextSize,
) -> Option<LuaType> {
    // Static members already have assignment-aware flow; do not replace their
    // more precise value with the collection's declared element contract.
    if model
        .resolve_member(index)
        .is_some_and(|member| member.member_id.is_some())
    {
        return None;
    }
    let access = super::computed_guard::access(model, &LuaExpr::IndexExpr(index.clone()))?;
    let prefix = index.get_prefix_expr()?;
    let owner_type = model.type_of_expr_at(prefix.get_syntax_id(), offset);
    if let LuaType::Object(object) = &owner_type
        && matches!(access.parts.last(), Some(super::computed_guard::Part::Member(key)) if object.get_fields().contains_key(key))
    {
        return None;
    }
    // An arbitrary class can transform writes through __newindex independently
    // of reads through __index. Only use ordinary array/dictionary contracts.
    if !super::computed_guard::ordinary_collection(model, &owner_type) {
        return None;
    }
    // This is the raw expression cache, distinct from the flow-sensitive query
    // that called us. Recursive raw inference can still yield Unknown; only a
    // concrete owner array contract supplies a conservative element fallback.
    let inferred = model.type_of_expr(index.get_syntax_id());
    let base = match (&inferred, &owner_type) {
        // Recursive element inference can be unknown at a loop/write site. The
        // owner's array contract is still available, as in guarded_index_type.
        (LuaType::Unknown, LuaType::Array(array)) => {
            LuaType::from_vec(vec![array.get_base().clone(), LuaType::Nil])
        }
        _ => inferred,
    };
    // An assignment can prove presence only for a usable element contract.
    // Do not replace normal member resolution with an uninformative proof.
    if matches!(base, LuaType::Unknown | LuaType::Any | LuaType::Never) {
        return None;
    }
    // Keep the assignment proof even when the raw value contract omits nil.
    // A downstream dictionary read can add absence to that same contract.
    if computed_effects::calls_conflict(
        model,
        &access,
        index.ancestors::<LuaStat>().next()?.syntax(),
        offset,
    ) {
        return None;
    }
    let closure = crate::semantic_db::flow::LuaClosureId::from_node(index.syntax());
    let tree = model.flow_tree()?;
    let root = model.chunk()?;
    let mut current = tree.get_flow_id_at(offset)?;
    // Bounded traversal also protects incomplete or cyclic control-flow graphs.
    for _ in 0..MAX_ASSIGNMENT_TRACE_STEPS {
        let node = tree.get_flow_node(current)?;
        match &node.kind {
            FlowNodeKind::Assignment(ptr) => {
                let assignment = ptr.to_node(&root)?;
                if assignment.get_range().end() <= offset {
                    if crate::semantic_db::flow::LuaClosureId::from_node(assignment.syntax())
                        != closure
                    {
                        return None;
                    }
                    let (vars, values) = assignment.get_var_and_expr_list();
                    if vars.len() == 1
                        && values.len() == 1
                        && let LuaVarExpr::IndexExpr(target) = &vars[0]
                        && super::computed_guard::access(model, &LuaExpr::IndexExpr(target.clone()))
                            .as_ref()
                            == Some(&access)
                    {
                        let value = &values[0];
                        // The completed write replaces effects on the entry itself.
                        // Receiver/key changes during RHS evaluation still prevent
                        // this write from proving the later syntactic access.
                        if computed_effects::assignment_inputs_conflict(
                            model,
                            &access,
                            assignment.syntax(),
                            offset,
                        ) {
                            return None;
                        }
                        let assigned =
                            model.type_of_expr_at(value.get_syntax_id(), value.get_position());
                        // Preserve a compatible declared element contract instead
                        // of narrowing to the constructor's initialization shape.
                        // An invalid write is diagnosed separately and cannot
                        // establish a valid collection-element proof here.
                        let contract = super::remove_type(base, &LuaType::Nil);
                        return assignment_proves_contract(model, &assigned, &contract)
                            .then_some(contract);
                    }
                    if computed_effects::calls_conflict(model, &access, assignment.syntax(), offset)
                    {
                        return None;
                    }
                    if vars
                        .iter()
                        .any(|var| computed_effects::write_conflicts(model, &access, var, offset))
                    {
                        return None;
                    }
                }
            }
            FlowNodeKind::CallExprStat(ptr) => {
                let call = LuaExpr::CallExpr(ptr.to_node(&root)?.get_call_expr()?);
                if computed_effects::calls_conflict(model, &access, call.syntax(), offset) {
                    return None;
                }
            }
            FlowNodeKind::NamedLabel(_) | FlowNodeKind::ImplFunc(_) => {
                return None;
            }
            FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                if computed_effects::calls_conflict(
                    model,
                    &access,
                    ptr.to_node(&root)?.syntax(),
                    offset,
                ) {
                    return None;
                }
            }
            _ => {}
        }
        for effect in tree.get_flow_effects(current) {
            if let FlowEffect::LoopEntry(syntax) = effect {
                let loop_node = syntax.to_node_from_root(root.syntax())?;
                if computed_effects::loop_conflicts(model, &access, &loop_node) {
                    return None;
                }
            }
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                if value_syntax.get_range().end() <= offset && access.depends_on(decl) {
                    return None;
                }
                let value = LuaExpr::cast(value_syntax.to_node_from_root(root.syntax())?)?;
                if computed_effects::calls_conflict(model, &access, value.syntax(), offset) {
                    return None;
                }
            }
        }
        let Some(FlowAntecedent::Single(next)) = node.antecedent else {
            return None;
        };
        current = next;
    }
    None
}

/// Every possible assigned value must be present and satisfy the element contract.
fn assignment_proves_contract(
    model: &SemanticModel,
    assigned: &LuaType,
    contract: &LuaType,
) -> bool {
    match assigned {
        LuaType::Nil | LuaType::Any | LuaType::Unknown | LuaType::Never => false,
        LuaType::Union(union) => union
            .into_vec()
            .iter()
            .all(|value| assignment_proves_contract(model, value, contract)),
        _ => model.type_check(assigned, contract),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stable_path_tests;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod uncertain_base_tests;

#[cfg(test)]
mod metamethod_tests;

#[cfg(test)]
mod rhs_order_tests;
