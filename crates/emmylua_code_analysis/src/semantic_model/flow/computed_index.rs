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
    let inferred = model.type_of_expr(index.get_syntax_id());
    let base = match (&inferred, &owner_type) {
        // Recursive element inference can be unknown at a loop/write site. The
        // owner's array contract is still available, as in guarded_index_type.
        (LuaType::Unknown, LuaType::Array(array)) => {
            LuaType::from_vec(vec![array.get_base().clone(), LuaType::Nil])
        }
        _ => inferred,
    };
    // Later index-read policies can add nil to a non-nullable inference seed.
    // Still require a definite write; its proof removes only nil from the seed.
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
    fn definite_assignment_proof_survives_a_nonnullable_inference_seed() {
        let write = "inventory[key] = inventory[key] or {count = 0}";
        for (initialization, effect, proven) in [
            ("", "", false),
            (write, "", true),
            (write, "inventory[key] = nil", false),
            (write, "inventory = {}", false),
            (write, "key = other", false),
            ("if flag then inventory[key] = {count = 0} end", "", false),
            ("inventory[other] = {count = 0}", "", false),
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            let mut config = ws.get_emmyrc();
            config.strict.array_index = false;
            ws.update_emmyrc(config);
            let file = ws.def(&format!(
                r#"
                ---@class Item
                ---@field count integer
                local Item = {{}}
                ---@param key integer
                ---@param other integer
                local function update(key, other)
                    ---@type Item[]
                    local inventory = {{}}
                    {initialization}
                    {effect}
                    local observed = inventory[key]
                end
                "#
            ));
            let model = ws.analysis.semantic_model(file);
            let read = model
                .chunk()
                .unwrap()
                .descendants::<LuaIndexExpr>()
                .filter(|index| index.get_text() == "inventory[key]")
                .last()
                .unwrap();
            let seed = model.type_of_expr(read.get_syntax_id());
            assert!(!seed.is_nullable(), "{seed:?}");
            let proof = assigned_type(&model, &read, read.get_position());
            assert_eq!(
                proof.is_some(),
                proven,
                "initialization={initialization:?}, effect={effect:?}: {proof:?}"
            );
            assert_eq!(
                model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
                seed,
                "ordinary read changed: initialization={initialization:?}, effect={effect:?}"
            );
            if let Some(proof) = proof {
                assert_eq!(ws.humanize_type(proof), "Item");
            }
        }
    }

    #[test]
    fn local_dictionary_initialization_has_no_nil_diagnostic() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        // Use public configuration so this fixture also covers the separately
        // maintained dictionary-read policy when the fixes are combined.
        let mut config = serde_json::to_value(ws.get_emmyrc()).unwrap();
        config["strict"]["tableIndex"] = serde_json::json!(true);
        ws.update_emmyrc(serde_json::from_value(config).unwrap());
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Item
            ---@field count integer
            local Item = {}
            ---@alias Inventory table<string, Item>
            ---@param amounts table<string, integer>
            local function update(amounts)
                ---@type Inventory
                local inventory = {}
                for key, amount in pairs(amounts) do
                    if amount > 0 then
                        inventory[key] = inventory[key] or {count = 0} --[[@as Item]]
                        inventory[key].count = inventory[key].count + amount
                    else
                        inventory[key] = nil
                    end
                end
            end
            "#,
        ));
    }

    #[test]
    fn conditional_and_other_key_writes_do_not_suppress_nil_diagnostics() {
        for (collection, key_type) in [("Item[]", "integer"), ("table<string, Item>", "string")] {
            for write in [
                "",
                "if flag then inventory[key] = {count = 0} end",
                "inventory[other] = {count = 0}",
            ] {
                let mut ws = VirtualWorkspace::new_with_init_std_lib();
                let mut config = ws.get_emmyrc();
                config.strict.array_index = true;
                let mut config = serde_json::to_value(config).unwrap();
                // The isolated base has no dictionary policy/configuration field.
                // Its serialized schema identifies whether dictionary nil checks
                // exist; an unknown tableIndex option is otherwise ignored.
                let dictionary_checks = config["strict"].get("tableIndex").is_some();
                if collection != "Item[]" {
                    config["strict"]["tableIndex"] = serde_json::json!(true);
                }
                ws.update_emmyrc(serde_json::from_value(config).unwrap());
                let source = format!(
                    r#"
                    ---@class Item
                    ---@field count integer
                    local Item = {{}}
                    ---@param key {key_type}
                    ---@param other {key_type}
                    ---@param flag boolean
                    local function update(key, other, flag)
                        ---@type {collection}
                        local inventory = {{}}
                        {write}
                        return inventory[key].count
                    end
                    "#,
                );
                let no_nil_diagnostic = ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source);
                if collection == "Item[]" || dictionary_checks {
                    assert!(
                        !no_nil_diagnostic,
                        "missing nil warning: {collection}, {write}"
                    );
                } else {
                    assert!(
                        no_nil_diagnostic,
                        "unexpected dictionary policy on isolated base"
                    );
                }
            }
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

#[cfg(test)]
#[test]
fn global_collection_reset_invalidates_prior_write() {
    for (effect, nullable, local_read) in [
        ("storage.ConnectedSurfaces={}", true, false),
        ("storage={ConnectedSurfaces={}}", true, false),
        ("other.ConnectedSurfaces={}", false, false),
        (
            "local storage={}; storage.ConnectedSurfaces={}",
            false,
            false,
        ),
        ("", false, false),
        ("storage.ConnectedSurfaces={}", false, true),
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let mut config = ws.get_emmyrc();
        config.strict.array_index = true;
        ws.update_emmyrc(config);
        ws.def_file("types.lua", "---@class Storage\n---@field ConnectedSurfaces integer[][]\n---@type Storage\nstorage={ConnectedSurfaces={}}\n---@type Storage\nother={ConnectedSurfaces={}}");
        let read_binding = if local_read {
            "---@type Storage\nlocal storage={ConnectedSurfaces={}}\n"
        } else {
            ""
        };
        let source = format!(
            "local function clear() {effect} end\n{read_binding}---@param key integer\n---@param entity_key integer\nlocal function connect(key,entity_key)\n storage.ConnectedSurfaces[key]=storage.ConnectedSurfaces[key] or {{}}\n clear()\n local surface_connections=storage.ConnectedSurfaces[key]\n surface_connections[entity_key]=1\nend\nreturn connect"
        );
        let file = ws.def_file("case.lua", &source);
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .filter(|i| i.get_text() == "storage.ConnectedSurfaces[key]")
            .last()
            .unwrap();
        let call = chunk
            .descendants::<emmylua_parser::LuaCallExpr>()
            .find(|call| call.get_text() == "clear()")
            .unwrap();
        assert!(index.get_position() > call.get_range().end());
        let access =
            super::computed_guard::access(&model, &LuaExpr::IndexExpr(index.clone())).unwrap();
        if local_read {
            assert!(matches!(&access.root, crate::SemanticId::Decl(_)));
            assert_ne!(Some(access.root.clone()), model.global_decl("storage"));
        }
        assert_eq!(
            computed_effects::calls_conflict(
                &model,
                &access,
                call.syntax(),
                call.get_range().end()
            ),
            nullable,
            "visible effect: {effect}"
        );
        assert_eq!(
            model
                .type_of_expr_at(index.get_syntax_id(), index.get_position())
                .is_nullable(),
            nullable,
            "{effect}"
        );
    }
}

#[cfg(test)]
#[test]
fn unresolved_global_call_effect_has_no_declaration_identity() {
    for (effect, conflicts) in [
        ("storage.ConnectedSurfaces = {}", false),
        ("other.ConnectedSurfaces = {}", false),
        ("local storage = {}; storage.ConnectedSurfaces = {}", false),
        ("", false),
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(&format!(
            "local function clear() {effect} end\nlocal key = 'x'\nclear()\nlocal observed = storage.ConnectedSurfaces[key]"
        ));
        let model = ws.analysis.semantic_model(file);
        // Without an indexed declaration, the resolver cannot identify this owner.
        assert!(model.global_decl("storage").is_none());
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "storage.ConnectedSurfaces[key]")
            .unwrap();
        let call = chunk
            .descendants::<emmylua_parser::LuaCallExpr>()
            .find(|call| call.get_text() == "clear()")
            .unwrap();
        assert!(index.get_position() > call.get_range().end());
        let access = super::computed_guard::access(&model, &LuaExpr::IndexExpr(index)).unwrap();
        assert!(
            matches!(&access.root, crate::SemanticId::Name(name) if name.as_str() == "storage")
        );
        assert_eq!(
            computed_effects::calls_conflict(
                &model,
                &access,
                call.syntax(),
                call.get_range().end()
            ),
            conflicts,
            "{effect}"
        );
    }
}
