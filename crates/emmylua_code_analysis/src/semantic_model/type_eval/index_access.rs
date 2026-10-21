//! Broad keys select complete index projections using precise subtype matching.
use super::*;
use crate::semantic_model::cache::QueryCacheScope;

pub(super) fn dynamic_index_type(
    model: &SemanticModel,
    base: &LuaType,
    key: &LuaType,
) -> Option<LuaType> {
    let _query = QueryCacheScope::new(&model.cache);
    let revision = model.cache.borrow().revision();
    let infos = model.index_infos(base);
    {
        let cache = model.cache.borrow();
        if cache.evaluation_exhausted || !cache.is_complete(revision) {
            return None;
        }
    }
    let mut types = Vec::new();
    for info in infos {
        // Ordinary compatibility can accept a broad string for a literal domain.
        // RawGet needs a precise domain proof, and incomplete checks must retry.
        if model.type_check_cached(key, &info.key_type, true)? {
            types.push(info.value_type);
        }
    }
    if types.is_empty() {
        return None;
    }
    // A broad runtime key does not prove the selected entry exists.
    types.push(LuaType::Nil);
    Some(LuaType::from_vec(types))
}
