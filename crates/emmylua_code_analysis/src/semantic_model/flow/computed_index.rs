//! Narrow reads through a stable local key after a definite write.
//!
//! Computed keys do not have a static member ID. Track the declarations of the
//! table and key instead. Stop at conflicting effects and uncertain joins;
//! preserve facts across opaque calls and loops without visible conflicting writes.

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
    // An arbitrary class can transform writes through __newindex independently
    // of reads through __index. Only use ordinary array/dictionary contracts.
    if !super::computed_guard::ordinary_collection(model, &owner_type) {
        return None;
    }
    let inferred = model.type_of_expr(index.get_syntax_id());
    let base = match (&inferred, &owner_type) {
        // Recursive element inference can be unknown at a loop/write site. The
        // owner's array contract is still available, as in guarded_index_type.
        (LuaType::Unknown, LuaType::Array(array)) => {
            LuaType::from_vec(vec![array.get_base().clone(), LuaType::Nil])
        }
        _ => inferred,
    };
    if !base.is_nullable() {
        return None;
    }
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
                    if computed_effects::calls_conflict(model, &access, assignment.syntax(), offset)
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
                        if computed_effects::calls_conflict(model, &access, value.syntax(), offset)
                        {
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
    fn computed_index_assignment_survives_opaque_calls() {
        for body in [
            "groups[key] = {}; mutate(); local values = groups[key].values",
            "groups[key] = {}; local unused = mutate(); local values = groups[key].values",
            "groups[key] = {}; if mutate() then local values = groups[key].values end",
            "groups[key] = {}; print(mutate(), groups[key].values)",
            "groups[key] = {}; local unused, values = mutate(), groups[key].values",
        ] {
            check(body, true);
        }
    }
}

#[cfg(test)]
mod stable_path_tests {
    use crate::{DiagnosticCode, VirtualWorkspace};
    fn check(body: &str, safe: bool) {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let src = format!(
            r#"
---@class Group
---@field values integer[]
---@alias Groups Group[]
---@class Storage
---@field groups Group[]
---@param storage Storage
---@param other_storage Storage
---@param key integer
---@param other integer
---@param value Group?
local function f(storage, other_storage, key, other, value)
{body}
end
"#
        );
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &src),
            safe,
            "{body}"
        );
    }
    #[test]
    fn stable_nested_assignment_paths() {
        for body in [
            "storage.groups[key] = {}; mutate(); print(storage.groups[key].values)",
            "storage.groups[key] = storage.groups[key] or {}; local result = storage.groups[key]; print(result.values)",
            "local groups = storage.groups; groups[key] = groups[key] or {}; print(groups[key].values)",
            "storage.groups[key] = {}; local count = #storage.groups[key].values",
        ] {
            check(body, true);
        }
    }
    #[test]
    fn stable_nested_assignment_invalidations() {
        for body in [
            "storage.groups[key] = {}; key = other; print(storage.groups[key].values)",
            "storage.groups[key] = {}; storage.groups = {}; print(storage.groups[key].values)",
            "storage.groups[key] = {}; storage = other_storage; print(storage.groups[key].values)",
            "storage.groups[key] = value; print(storage.groups[key].values)",
            "if key > 0 then storage.groups[key] = {} end; print(storage.groups[key].values)",
        ] {
            check(body, false);
        }
    }
}
