use super::*;
use std::cell::Cell;

thread_local! {
    static CLASSIFICATIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn record_classification() {
    CLASSIFICATIONS.set(CLASSIFICATIONS.get() + 1);
}

fn call(model: &SemanticModel, text: &str) -> LuaCallExpr {
    model
        .chunk()
        .unwrap()
        .descendants::<LuaCallExpr>()
        .find(|call| call.get_text() == text)
        .unwrap()
}

#[test]
fn cached_classification_preserves_receiver_key_and_binding_checks() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def("local receiver = {}; local other = {}; local function apply(target) rawset(target, 'value', nil) end; table.insert(receiver, 1); local a = receiver.value; local b = other.value; local c = receiver.sibling");
    let model = ws.analysis.semantic_model(file);
    let path = |text| {
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == text)
            .unwrap();
        index_access_path(&model, &LuaExpr::IndexExpr(index)).unwrap()
    };
    let receiver = path("receiver.value");
    let other = path("other.value");
    let sibling = path("receiver.sibling");
    let rawset = call(&model, "rawset(target, 'value', nil)");
    let target = rawset.get_args_list().unwrap().get_args().next().unwrap();
    let formal = index_access_path(&model, &target).unwrap().0;
    CLASSIFICATIONS.set(0);
    for _ in 0..32 {
        let bindings = vec![(formal.clone(), vec![(receiver.0.clone(), Vec::new())])];
        assert!(mutator_conflicts(&model, &receiver, &rawset, &bindings));
        assert!(!mutator_conflicts(&model, &other, &rawset, &bindings));
        assert!(!mutator_conflicts(&model, &sibling, &rawset, &bindings));
        let bindings = vec![(formal.clone(), vec![(other.0.clone(), Vec::new())])];
        assert!(!mutator_conflicts(&model, &receiver, &rawset, &bindings));
        assert!(mutator_conflicts(&model, &other, &rawset, &bindings));
        let insert = call(&model, "table.insert(receiver, 1)");
        assert!(mutator_conflicts(
            &model,
            &receiver,
            &insert,
            &Bindings::new()
        ));
        assert!(!mutator_conflicts(
            &model,
            &other,
            &insert,
            &Bindings::new()
        ));
    }
    assert_eq!(CLASSIFICATIONS.get(), 2);
}

#[test]
fn classifications_isolate_shared_file_views_and_edits() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let source = |name| format!("local action = {name}; action({{}}, 'value', nil)");
    let first = ws.def_file("first.lua", &source("rawset"));
    let second = ws.def_file("second.lua", &source("normal"));
    let check = |model: &SemanticModel, expected| {
        let prefix = call(model, "action({}, 'value', nil)")
            .get_prefix_expr()
            .unwrap();
        assert_eq!(mutator_kinds(model, &prefix).rawset, expected);
    };
    {
        let root = ws.analysis.semantic_model(first);
        let other = root.model_for(second);
        for _ in 0..2 {
            check(&root, true);
            check(&other, false);
        }
    }
    assert_eq!(ws.def_file("first.lua", &source("normal")), first);
    check(&ws.analysis.semantic_model(first), false);
}

#[test]
fn incomplete_classification_is_retried_after_the_query() {
    use crate::semantic_model::cache::{CacheEntry, QueryCacheScope};
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def("rawset({}, 'value', nil)");
    let model = ws.analysis.semantic_model(file);
    let prefix = call(&model, "rawset({}, 'value', nil)")
        .get_prefix_expr()
        .unwrap();
    let key = (file, prefix.get_syntax_id());
    {
        let _outer = QueryCacheScope::new(&model.cache);
        let revision = model.cache.borrow().revision();
        model
            .cache
            .borrow_mut()
            .member_mutator_kinds
            .insert(key, CacheEntry::InProgress);
        assert!(!mutator_kinds(&model, &prefix).rawset);
        assert!(!model.cache.borrow().is_complete(revision));
    }
    assert!(!model.cache.borrow().member_mutator_kinds.contains_key(&key));
    assert!(mutator_kinds(&model, &prefix).rawset);
}
