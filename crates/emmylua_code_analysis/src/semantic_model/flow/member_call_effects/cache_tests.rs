use super::*;
use std::cell::Cell;

thread_local! {
    static RESOLUTIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn record_resolution() {
    RESOLUTIONS.set(RESOLUTIONS.get() + 1);
}

fn index(model: &SemanticModel, text: &str) -> LuaIndexExpr {
    model
        .chunk()
        .unwrap()
        .descendants::<LuaIndexExpr>()
        .find(|index| index.get_text() == text)
        .unwrap()
}

fn call(model: &SemanticModel, text: &str) -> LuaCallExpr {
    model
        .chunk()
        .unwrap()
        .descendants::<LuaCallExpr>()
        .find(|call| call.get_text() == text)
        .unwrap()
}

fn conflict(model: &SemanticModel, guarded: &str, invoked: &str) -> bool {
    let call = call(model, invoked);
    calls_conflict(
        model,
        &index(model, guarded),
        call.syntax(),
        call.get_range().end(),
    )
}

#[test]
fn many_guarded_members_share_callee_resolution() {
    let mut ws = crate::VirtualWorkspace::new();
    let reads = (0..32)
        .map(|n| format!("local read{n} = receiver.value{n}\n"))
        .collect::<String>();
    let file = ws.def(&format!(
        "local receiver = {{}}; local methods = {{write = function(target) target.value0 = nil end}}; methods.write(receiver); {reads}"
    ));
    let model = ws.analysis.semantic_model(file);
    RESOLUTIONS.set(0);
    for _ in 0..2 {
        for n in 0..32 {
            assert_eq!(
                conflict(
                    &model,
                    &format!("receiver.value{n}"),
                    "methods.write(receiver)"
                ),
                n == 0
            );
        }
    }
    // Count actual resolver work instead of relying on machine-dependent timings.
    assert_eq!(
        RESOLUTIONS.get(),
        1,
        "resolved one unchanged call {} times",
        RESOLUTIONS.get()
    );
}

#[test]
fn effects_keep_receiver_body_and_time_boundaries() {
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local receiver = {}; local other = {}; local function write(target) target.value = nil end; local function harmless() end; write(receiver); harmless(); local a = receiver.value; local b = other.value; local c = receiver.sibling");
    let model = ws.analysis.semantic_model(file);
    let guarded = index(&model, "receiver.value");
    let write = call(&model, "write(receiver)");
    for _ in 0..2 {
        assert!(!calls_conflict(
            &model,
            &guarded,
            write.syntax(),
            write.get_range().start()
        ));
        assert!(conflict(&model, "receiver.value", "write(receiver)"));
        assert!(!calls_conflict_after(
            &model,
            &guarded,
            write.syntax(),
            write.get_range().end(),
            write.get_range().end()
        ));
        assert!(!conflict(&model, "other.value", "write(receiver)"));
        assert!(!conflict(&model, "receiver.sibling", "write(receiver)"));
        assert!(!conflict(&model, "receiver.value", "harmless()"));
    }
}

#[test]
fn shared_file_views_and_edits_keep_distinct_effects() {
    // Equal body widths keep call syntax ranges identical across both files.
    let source = |body| {
        format!(
            "local receiver = {{}}; local methods = {{write = function(target) {body: <24} end}}; methods.write(receiver); local observed = receiver.value"
        )
    };
    let writes = source("target.value = nil");
    let harmless = source("local unused = 1");
    let mut ws = crate::VirtualWorkspace::new();
    let first = ws.def_file("first.lua", &writes);
    let second = ws.def_file("second.lua", &harmless);
    {
        let root = ws.analysis.semantic_model(first);
        let other = root.model_for(second);
        assert_eq!(
            call(&root, "methods.write(receiver)").get_range(),
            call(&other, "methods.write(receiver)").get_range()
        );
        for _ in 0..2 {
            assert!(conflict(&root, "receiver.value", "methods.write(receiver)"));
            assert!(!conflict(
                &other,
                "receiver.value",
                "methods.write(receiver)"
            ));
        }
    }
    assert_eq!(ws.def_file("first.lua", &harmless), first);
    let edited = ws.analysis.semantic_model(first);
    assert!(!conflict(
        &edited,
        "receiver.value",
        "methods.write(receiver)"
    ));
}

#[test]
fn callee_bindings_keep_distinct_invocation_effects() {
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local receiver = {}; local writer = {run = function(target) target.value = nil end}; local quiet = {run = function(target) end}; local function invoke(methods, target) methods.run(target) end; invoke(writer, receiver); invoke(quiet, receiver); local observed = receiver.value");
    let model = ws.analysis.semantic_model(file);
    for _ in 0..2 {
        assert!(conflict(
            &model,
            "receiver.value",
            "invoke(writer, receiver)"
        ));
        assert!(!conflict(
            &model,
            "receiver.value",
            "invoke(quiet, receiver)"
        ));
    }
}

#[test]
fn incomplete_callee_resolution_is_retried_after_the_query() {
    use crate::semantic_model::cache::{CacheEntry, QueryCacheScope};

    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local receiver = {}; local methods = {write = function(target) target.value = nil end}; methods.write(receiver); local observed = receiver.value");
    let model = ws.analysis.semantic_model(file);
    assert!(conflict(
        &model,
        "receiver.value",
        "methods.write(receiver)"
    ));
    let key = model
        .cache
        .borrow()
        .member_callees
        .keys()
        .next()
        .unwrap()
        .clone();
    {
        let _outer = QueryCacheScope::new(&model.cache);
        {
            let mut cache = model.cache.borrow_mut();
            cache.member_call_effects.clear();
            cache
                .member_callees
                .insert(key.clone(), CacheEntry::InProgress);
        }
        // A recursive unresolved read must not permanently cache "no visible write".
        assert!(!conflict(
            &model,
            "receiver.value",
            "methods.write(receiver)"
        ));
    }
    assert!(!model.cache.borrow().member_callees.contains_key(&key));
    assert!(conflict(
        &model,
        "receiver.value",
        "methods.write(receiver)"
    ));
}

#[test]
fn receiver_target_order_does_not_change_visible_effects() {
    use crate::semantic_model::cache::CacheEntry;

    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local receiver = {}; local writer = {run = function(target) target.value = nil end}; local quiet = {run = function(target) end}; local function invoke(methods, target) methods.run(target) end; invoke(writer, receiver); invoke(quiet, receiver); local observed = receiver.value");
    let model = ws.analysis.semantic_model(file);
    let targets = fixed_receiver_targets(&model);
    assert!(targets.len() > 2);
    for rotation in 0..targets.len() {
        let mut reordered = targets.as_ref().clone();
        reordered.rotate_left(rotation);
        {
            let mut cache = model.cache.borrow_mut();
            cache
                .member_receiver_targets
                .insert(file, CacheEntry::Ready(Rc::new(reordered)));
            cache.member_callees.clear();
            cache.member_call_effects.clear();
        }
        assert!(conflict(
            &model,
            "receiver.value",
            "invoke(writer, receiver)"
        ));
        assert!(!conflict(
            &model,
            "receiver.value",
            "invoke(quiet, receiver)"
        ));
    }
}

#[test]
fn incomplete_receiver_targets_are_retried_after_the_query() {
    use crate::semantic_model::cache::{CacheEntry, QueryCacheScope};

    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local receiver = {}; local methods = {write = function(target) target.value = nil end}; methods.write(receiver); local observed = receiver.value");
    let model = ws.analysis.semantic_model(file);
    {
        let _outer = QueryCacheScope::new(&model.cache);
        let revision = model.cache.borrow().revision();
        model
            .cache
            .borrow_mut()
            .member_receiver_targets
            .insert(file, CacheEntry::InProgress);
        assert!(!conflict(
            &model,
            "receiver.value",
            "methods.write(receiver)"
        ));
        assert!(!model.cache.borrow().is_complete(revision));
    }
    assert!(
        !model
            .cache
            .borrow()
            .member_receiver_targets
            .contains_key(&file)
    );
    assert!(conflict(
        &model,
        "receiver.value",
        "methods.write(receiver)"
    ));
}

#[test]
fn recursive_callees_keep_visible_writes_and_harmless_calls_distinct() {
    for (effect, writes) in [("target.value = nil", true), ("", false)] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            "local receiver = {{}}; local function effect(target) {effect} end; local function recurse(target) if flag then recurse(target) end; effect(target) end; recurse(receiver); local observed = receiver.value"
        ));
        let model = ws.analysis.semantic_model(file);
        for _ in 0..2 {
            assert_eq!(
                conflict(&model, "receiver.value", "recurse(receiver)"),
                writes
            );
        }
    }
}
