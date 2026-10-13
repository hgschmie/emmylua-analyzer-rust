//! Narrow reads through a stable local key after a definite write.
//!
//! Computed keys do not have a static member ID. Track the declarations of the
//! table and key instead, within straight-line flow. Stop at mutations, calls,
//! joins and loops rather than carrying a fact across a possibly different key.

use emmylua_parser::{
    LuaAstNode, LuaExpr, LuaIndexExpr, LuaIndexKey, LuaStat, LuaSyntaxKind, LuaSyntaxNode,
    LuaVarExpr,
};
use rowan::TextSize;

use super::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaType, SemanticId, SemanticModel};

pub(super) fn assigned_type(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    offset: TextSize,
) -> Option<LuaType> {
    let (owner, key) = local_index(model, index)?;
    let prefix = index.get_prefix_expr()?;
    let owner_type = model.type_of_expr_at(prefix.get_syntax_id(), offset);
    // An arbitrary class can transform writes through __newindex independently
    // of reads through __index. Only use ordinary array/dictionary contracts.
    match &owner_type {
        LuaType::Array(_) | LuaType::TableGeneric(_) => {}
        LuaType::Generic(generic)
            if generic.get_base_type_id() == crate::LuaTypeDeclId::global("table")
                && model
                    .type_def_of(&generic.get_base_type_id())
                    .is_some_and(|def| model.is_std_file(def.file_id)) => {}
        _ => return None,
    }
    let base = model.type_of_expr(index.get_syntax_id());
    if !base.is_nullable() {
        return None;
    }
    if contains_call(index.ancestors::<LuaStat>().next()?.syntax(), offset) {
        return None;
    }
    let closure = crate::semantic_db::flow::LuaClosureId::from_node(index.syntax());
    let tree = model.flow_tree()?;
    let root = model.chunk()?;
    let mut current = tree.get_flow_id_at(offset)?;
    // Bounded traversal also protects incomplete or cyclic control-flow graphs.
    for _ in 0..128 {
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
                        && local_index(model, target).as_ref()
                            == Some(&(owner.clone(), key.clone()))
                    {
                        let value = &values[0];
                        if contains_call(value.syntax(), offset) {
                            return None;
                        }
                        let assigned =
                            model.type_of_expr_at(value.get_syntax_id(), value.get_position());
                        // Preserve the declared element type, not the narrower
                        // shape of a constructor used to initialize that element.
                        return (!assigned.is_nullable()
                            && !matches!(
                                assigned,
                                LuaType::Any | LuaType::Unknown | LuaType::Never
                            ))
                        .then(|| super::remove_type(base, &LuaType::Nil));
                    }
                    // Any indexed write might alias this table or key. Multiple
                    // assignment also evaluates keys before replacing locals.
                    if vars
                        .iter()
                        .any(|var| matches!(var, LuaVarExpr::IndexExpr(_)))
                    {
                        return None;
                    }
                }
            }
            FlowNodeKind::CallExprStat(ptr) => {
                let call = LuaExpr::CallExpr(ptr.to_node(&root)?.get_call_expr()?);
                if contains_call(call.syntax(), offset) {
                    return None;
                }
            }
            FlowNodeKind::LoopLabel | FlowNodeKind::NamedLabel(_) | FlowNodeKind::ImplFunc(_) => {
                return None;
            }
            FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                if contains_call(ptr.to_node(&root)?.syntax(), offset) {
                    return None;
                }
            }
            _ => {}
        }
        for effect in tree.get_flow_effects(current) {
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                if value_syntax.get_range().end() <= offset && (decl == &owner || decl == &key) {
                    return None;
                }
                let value = LuaExpr::cast(value_syntax.to_node_from_root(root.syntax())?)?;
                if contains_call(value.syntax(), offset) {
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

fn local_index(model: &SemanticModel, index: &LuaIndexExpr) -> Option<(SemanticId, SemanticId)> {
    let LuaExpr::NameExpr(owner) = index.get_prefix_expr()? else {
        return None;
    };
    let LuaIndexKey::Expr(LuaExpr::NameExpr(key)) = index.get_index_key()? else {
        return None;
    };
    let owner = model.resolve_name(owner.get_position())?;
    let key = model.resolve_name(key.get_position())?;
    let facts = model.file_facts()?;
    (facts.decl_by_id(&owner)?.kind.is_local() && facts.decl_by_id(&key)?.kind.is_local())
        .then_some((owner, key))
}

fn contains_call(node: &LuaSyntaxNode, before: TextSize) -> bool {
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        match node.kind().into() {
            LuaSyntaxKind::ClosureExpr => {}
            LuaSyntaxKind::CallExpr if node.text_range().end() <= before => return true,
            _ => pending.extend(node.children()),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagnosticCode, VirtualWorkspace};

    fn check(body: &str, safe: bool) {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            r#"
            ---@class Group
            ---@field values integer[]
            ---@param groups Group[]
            ---@param key integer
            ---@param other_key integer
            ---@param other_groups Group[]
            ---@param maybe Group?
            ---@param flag boolean
            local function add(groups, key, other_key, other_groups, maybe, flag)
                {body}
            end
        "#
        );
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            safe,
            "{body}"
        );
    }

    #[test]
    fn computed_index_assignment_narrows_read() {
        for body in [
            "groups[key] = groups[key] or { values = {} }; local values = groups[key].values",
            "groups[key] = { values = {} }; local values = groups[key].values",
            "groups[key] = { values = {} }; local unrelated = 1; local values = groups[key].values",
            "if flag then groups[key] = groups[key] or { values = {} }; local values = groups[key].values end",
            "for i = 1, 10 do groups[key] = groups[key] or { values = {} }; local values = groups[key].values end",
            "groups[key] = { values = {} }; print(groups[key].values)",
            "groups[key] = { values = {} }; return groups[key].values",
        ] {
            check(body, true);
        }
    }

    #[test]
    fn computed_index_assignment_preserves_element_type() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(
            r#"
            ---@class Group
            ---@field values integer[]
            ---@param groups Group[]
            ---@param key integer
            local function add(groups, key)
                groups[key] = groups[key] or { values = {} }
                local result = groups[key]
            end
        "#,
        );
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        let actual = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        assert_eq!(ws.humanize_type(actual), "Group");
    }

    #[test]
    fn computed_index_assignment_requires_same_stable_declarations() {
        for body in [
            "local values = groups[key].values",
            "groups[key] = {}; key = other_key; local values = groups[key].values",
            "groups[key] = {}; groups = other_groups; local values = groups[key].values",
            "groups[other_key] = {}; local values = groups[key].values",
            "other_groups[key] = {}; local values = groups[key].values",
            "groups[key] = {}; do local key = other_key; local values = groups[key].values end",
            "groups[key] = {}; do local groups = other_groups; local values = groups[key].values end",
            "groups[key], key = {}, other_key; local values = groups[key].values",
        ] {
            check(body, false);
        }
    }

    #[test]
    fn computed_index_assignment_does_not_cross_uncertain_writes_or_control_flow() {
        for body in [
            "groups[key] = {}; groups[key] = nil; local values = groups[key].values",
            "groups[key] = {}; groups[key] = maybe; local values = groups[key].values",
            "groups[key] = maybe; local values = groups[key].values",
            "if flag then groups[key] = {} end; local values = groups[key].values",
            "while flag do groups[key] = {} end; local values = groups[key].values",
            "local alias = groups; groups[key] = {}; alias[key] = nil; local values = groups[key].values",
            "groups[key] = {}; local function later() local values = groups[key].values end",
        ] {
            check(body, false);
        }
    }

    #[test]
    fn computed_index_assignment_stops_at_calls() {
        for body in [
            "groups[key] = {}; mutate(); local values = groups[key].values",
            "groups[key] = {}; local unused = mutate(); local values = groups[key].values",
            "groups[key] = {}; if mutate() then local values = groups[key].values end",
            "groups[key] = {}; print(mutate(), groups[key].values)",
            "groups[key] = {}; local unused, values = mutate(), groups[key].values",
        ] {
            check(body, false);
        }
    }
}
