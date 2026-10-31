//! Broad keys select complete index projections using precise subtype matching.
use super::SemanticModel;
use crate::LuaType;
use crate::semantic_model::cache::QueryCacheScope;

pub(super) fn dynamic_index_type(
    model: &SemanticModel,
    base: &LuaType,
    key: &LuaType,
) -> Option<LuaType> {
    // Nil is not a table key. Never is uninhabited, so there is nothing to project.
    if matches!(key, LuaType::Nil | LuaType::Never) {
        return None;
    }
    // Keep the outer query alive through projection and subtype proofs so partial
    // cache entries are discarded together. Nested scopes share the current outer
    // budget; only entering a fresh outer query resets it. eval_index_access's
    // AliasPath also holds an outer scope across all keys, so per-key calls cannot
    // reset exhaustion. index_infos reuses its cached projection between keys.
    // Keep the helper self-contained for direct callers as well.
    let _query = QueryCacheScope::new(&model.cache);
    if model.cache.borrow().evaluation_exhausted {
        return None;
    }
    // This counts incomplete queries, not cache insertions; opening the scope
    // does not increment it. Snapshot before any dependent query can fail.
    let revision = model.cache.borrow().revision();
    let infos = model.index_infos(base);
    {
        let cache = model.cache.borrow();
        // Nested queries share the outer budget. Even a cached complete contract
        // must not rescue an exhausted outer query; a later outer query can retry.
        if cache.evaluation_exhausted || !cache.is_complete(revision) {
            return None;
        }
    }
    let mut types = Vec::new();
    for info in infos {
        // Ordinary compatibility can accept a broad string for a literal domain.
        // Type-level indexing needs a precise domain proof. Any/Unknown retain
        // every possible domain's values; they never prove an entry exists.
        // An incomplete later proof must discard earlier matches as well.
        if model.type_check_cached(key, &info.key_type, true)? {
            types.push(info.value_type);
        }
    }
    if types.is_empty() {
        return None;
    }
    // A broad runtime key does not prove the selected entry exists. from_vec
    // normalizes the union, including nil already present in a value contract.
    types.push(LuaType::Nil);
    Some(LuaType::from_vec(types))
}
