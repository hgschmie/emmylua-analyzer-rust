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
