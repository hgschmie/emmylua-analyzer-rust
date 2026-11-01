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
