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
                        // Preserve a compatible declared element contract instead
                        // of narrowing to the constructor's initialization shape.
                        // An invalid write is diagnosed separately and cannot
                        // establish a valid collection-element proof here.
                        let contract = super::remove_type(base, &LuaType::Nil);
                        return assignment_proves_contract(model, &assigned, &contract)
                            .then_some(contract);
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

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn global_collection_reset_invalidates_prior_write() {
        for (effect, nullable) in [
            ("storage.ConnectedSurfaces={}", true),
            ("storage={ConnectedSurfaces={}}", true),
            ("other.ConnectedSurfaces={}", false),
            ("local storage={}; storage.ConnectedSurfaces={}", false),
            ("", false),
        ] {
            let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
            let mut config = ws.get_emmyrc();
            config.strict.array_index = true;
            ws.update_emmyrc(config);
            ws.def_file("types.lua", "---@class Storage\n---@field ConnectedSurfaces integer[][]\n---@type Storage\nstorage={ConnectedSurfaces={}}\n---@type Storage\nother={ConnectedSurfaces={}}");
            let source = format!(
                "local function clear() {effect} end\n---@param key integer\n---@param entity_key integer\nlocal function connect(key,entity_key)\n storage.ConnectedSurfaces[key]=storage.ConnectedSurfaces[key] or {{}}\n clear()\n local surface_connections=storage.ConnectedSurfaces[key]\n surface_connections[entity_key]=1\nend\nreturn connect"
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
            let access =
                super::super::computed_guard::access(&model, &LuaExpr::IndexExpr(index.clone()))
                    .unwrap();
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

    #[test]
    fn local_collection_initializer_in_iterator_keeps_nonnull_entry() {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let mut config = ws.get_emmyrc();
        config.strict.array_index = true;
        ws.update_emmyrc(config);
        assert!(ws.has_no_diagnostic(
            crate::DiagnosticCode::NeedCheckNil,
            r#"
---@class ItemCount
---@field count integer
---@alias InventoryType ItemCount[]
---@param shipment_items integer[]
local function test(shipment_items)
 ---@type InventoryType
 local inventory = {}
 for shipment, amount in pairs(shipment_items) do
   inventory[shipment] = inventory[shipment] or {count = 0} --[[@as ItemCount]]
   inventory[shipment].count = inventory[shipment].count + amount
 end
end
"#
        ));
    }

    #[test]
    fn incompatible_assignment_does_not_prove_the_element_contract() {
        for value in ["'oops'", "42", "false", "flag and 'oops' or {values={}}"] {
            let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
            let source = format!(
                "---@class Group\n---@field values integer[]\n\
                 ---@param groups Group[]\n---@param key integer\n---@param flag boolean\n\
                 local function test(groups, key, flag) groups[key] = {value}; local observed = groups[key].values end"
            );
            let file = ws.def(&source);
            let model = ws.analysis.semantic_model(file);
            let index = model
                .chunk()
                .unwrap()
                .descendants::<LuaIndexExpr>()
                .filter(|index| index.get_text() == "groups[key]")
                .last()
                .unwrap();
            for phase in ["cold", "warm"] {
                assert_eq!(
                    assigned_type(&model, &index, index.get_position()),
                    None,
                    "{value}: {phase}"
                );
            }
            assert!(
                !ws.has_no_diagnostic(crate::DiagnosticCode::NeedCheckNil, &source),
                "{value}"
            );
        }
    }

    #[test]
    fn dictionary_assignment_proof_keeps_declared_element_contract() {
        for (body, proved) in [
            ("inventory[key] = inventory[key] or {count=0}", true),
            ("inventory[key] = {count=0}; harmless()", true),
            ("", false),
            ("inventory[key] = nil", false),
            ("inventory[key] = {count=0}; key = other", false),
            ("inventory[key] = {count=0}; inventory = {}", false),
            ("if flag then inventory[key] = {count=0} end", false),
        ] {
            let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
            let file = ws.def(&format!(
                r#"
---@class ItemCount
---@field count integer
---@alias InventoryType table<string,ItemCount>
---@param key string
---@param other string
local function test(key, other)
 ---@type InventoryType
 local inventory = {{}}
 {body}
 local observed = inventory[key]
end
"#
            ));
            let model = ws.analysis.semantic_model(file);
            let index = model
                .chunk()
                .unwrap()
                .descendants::<LuaIndexExpr>()
                .last()
                .unwrap();
            let expected = proved.then(|| LuaType::Ref(crate::LuaTypeDeclId::global("ItemCount")));
            assert_eq!(
                assigned_type(&model, &index, index.get_position()),
                expected,
                "{body}"
            );
        }
    }
}

#[cfg(test)]
mod uncertain_base_tests {
    use super::*;
    use crate::semantic_model::cache::CacheEntry;

    #[test]
    fn assignment_proof_rejects_an_unusable_raw_element_type() {
        for raw in [LuaType::Unknown, LuaType::Any, LuaType::Never] {
            let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
            let file = ws.def("---@param values table<string,integer>\n---@param key string\nlocal function f(values, key) values[key] = 1; local observed = values[key] end");
            let model = ws.analysis.semantic_model(file);
            let index = model
                .chunk()
                .unwrap()
                .descendants::<LuaIndexExpr>()
                .last()
                .unwrap();
            model.cache.borrow_mut().expr_type.insert(
                (file, index.get_syntax_id()),
                CacheEntry::Ready(raw.clone()),
            );
            assert_eq!(
                assigned_type(&model, &index, index.get_position()),
                None,
                "{raw:?}"
            );
        }
    }
}

#[cfg(test)]
mod metamethod_tests {
    use super::*;

    #[test]
    fn custom_access_handlers_do_not_prove_assignment_presence() {
        for handler in ["", "__index", "__newindex"] {
            for anonymous in [false, true] {
                let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
                let extra = if handler.is_empty() {
                    String::new()
                } else {
                    format!(", {handler}: function")
                };
                let (definitions, owner) = if anonymous {
                    (String::new(), format!("{{ [string]: integer[]{extra} }}"))
                } else {
                    let field = if handler.is_empty() {
                        String::new()
                    } else {
                        format!("---@field {handler} function\n")
                    };
                    (
                        format!("---@class Rows: table<string, integer[]>\n{field}"),
                        "Rows".to_string(),
                    )
                };
                let source = format!(
                    "{definitions}---@param rows {owner}\n---@param key string\nlocal function f(rows,key) rows[key]={{1}}; local observed=rows[key] end"
                );
                let file = ws.def(&source);
                let contract = ws.ty(&owner);
                let model = ws.analysis.semantic_model(file);
                let index = model
                    .chunk()
                    .unwrap()
                    .descendants::<LuaIndexExpr>()
                    .filter(|index| index.get_text() == "rows[key]")
                    .last()
                    .unwrap();
                for phase in ["cold", "warm"] {
                    assert_eq!(
                        super::super::computed_guard::ordinary_collection(&model, &contract),
                        handler.is_empty(),
                        "classification anonymous={anonymous}, {handler}: {phase}"
                    );
                    if !anonymous || !handler.is_empty() {
                        assert_eq!(
                            assigned_type(&model, &index, index.get_position()).is_some(),
                            handler.is_empty(),
                            "assignment anonymous={anonymous}, {handler}: {phase}"
                        );
                    }
                }
            }
        }
    }
}
