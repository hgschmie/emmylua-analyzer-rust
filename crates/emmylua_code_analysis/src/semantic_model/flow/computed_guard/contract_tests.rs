use super::*;

#[test]
fn computed_collection_classification_rejects_partial_contracts() {
    let mut ws = crate::VirtualWorkspace::new();
    let file=ws.def("---@class Growing<T>: table<string,T>, Growing<T[]>\n---@field label string\n---@class Complete: table<string,integer>");
    let growing = ws.ty("Growing<integer>");
    let complete = ws.ty("Complete");
    let model = ws.analysis.semantic_model(file);
    for _ in 0..2 {
        assert!(!ordinary_collection(&model, &growing));
        assert!(!named_collection(
            &model,
            &growing,
            CollectionUse::ReadGuard
        ));
        assert!(ordinary_collection(&model, &complete));
        assert!(named_collection(
            &model,
            &complete,
            CollectionUse::ReadGuard
        ));
    }
}

#[test]
fn dictionary_guard_proof_keeps_declared_element_contract() {
    for (body, proved) in [
        (
            "if inventory[key] then local observed = inventory[key] end",
            true,
        ),
        (
            "if inventory[key] ~= nil then local observed = inventory[key] end",
            true,
        ),
        (
            "if type(inventory[key]) == 'table' then local observed = inventory[key] end",
            true,
        ),
        ("local observed = inventory[key]", false),
        (
            "if inventory[key] then inventory[key] = nil; local observed = inventory[key] end",
            false,
        ),
        (
            "if inventory[key] then inventory = {}; local observed = inventory[key] end",
            false,
        ),
        (
            "if inventory[key] then key = other; local observed = inventory[key] end",
            false,
        ),
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(&format!(
            r#"
---@class ItemCount
---@field count integer
---@alias InventoryType table<string,ItemCount>
---@param inventory InventoryType
---@param key string
---@param other string
local function test(inventory, key, other)
 {body}
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
            guarded_type(&model, &index, index.get_position()),
            expected,
            "{body}"
        );
    }
}
