use super::*;
use std::cell::Cell;

thread_local! {
    static BODIES: Cell<usize> = const { Cell::new(0) };
    static BODY_LIMIT: Cell<usize> = const { Cell::new(usize::MAX) };
    static RESOLUTIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn record_body() {
    BODIES.set(BODIES.get() + 1);
    assert!(
        BODIES.get() <= BODY_LIMIT.get(),
        "recursive effect scan exceeded {} bodies",
        BODY_LIMIT.get()
    );
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

#[test]
fn incomplete_effect_scan_is_retried_after_the_query() {
    use crate::semantic_model::cache::{CacheEntry, QueryCacheScope};

    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local receiver = {}; local function write(target) target.value = nil end; write(receiver); local observed = receiver.value");
    let model = ws.analysis.semantic_model(file);
    assert!(conflict(&model, "receiver.value", "write(receiver)"));
    let key = model
        .cache
        .borrow()
        .member_call_effects
        .keys()
        .next()
        .unwrap()
        .clone();
    {
        let _outer = QueryCacheScope::new(&model.cache);
        let revision = model.cache.borrow().revision();
        model
            .cache
            .borrow_mut()
            .member_call_effects
            .insert(key.clone(), CacheEntry::InProgress);
        assert!(!conflict(&model, "receiver.value", "write(receiver)"));
        assert!(!model.cache.borrow().is_complete(revision));
    }
    assert!(!model.cache.borrow().member_call_effects.contains_key(&key));
    assert!(conflict(&model, "receiver.value", "write(receiver)"));
}

#[test]
fn branching_recursive_calls_share_receiver_contexts() {
    struct RestoreLimit;
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            BODY_LIMIT.set(usize::MAX);
        }
    }
    let _limit = RestoreLimit;
    BODY_LIMIT.set(256);
    BODIES.set(0);
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(
        r#"
local external = {}
local function helper(value) external.log(value) end
local function build(parent, def)
    local element = parent.add(def)
    local left, middle, right = {}, {}, {}
    helper(left.name)
    helper(middle.name)
    helper(right.name)
    build(element, left)
    build(element, middle)
    build(element, right)
    element.style.width = 1
end
"#,
    );
    let model = ws.analysis.semantic_model(file);
    assert!(!conflict(&model, "element.style", "build(element, left)"));
    assert!(BODIES.get() > 1, "fixture must expand visible callees");
}

#[test]
fn recursive_member_replacements_reach_later_mutators() {
    for (member, writes) in [("value", true), ("other", false)] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
local receiver = {{value = 1}}
local methods = {{run = function(target) end, next = function(target) end,
    last = function(target) target.{member} = nil end}}
local function recurse(target)
    methods.run(target)
    methods.run = methods.next
    methods.next = methods.last
    recurse(target)
    recurse(target)
end
recurse(receiver)
local observed = receiver.value
"#
        ));
        let model = ws.analysis.semantic_model(file);
        assert_eq!(
            conflict(&model, "receiver.value", "recurse(receiver)"),
            writes
        );
    }
}

#[test]
fn recursive_member_calls_keep_receivers_and_callees_distinct() {
    for (method, receiver, writes) in [
        ("writer", "receiver", true),
        ("writer", "other", false),
        ("quiet", "receiver", false),
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
local receiver = {{value = 1}}
local other = {{value = 1}}
local writer = {{run = function(target) target.value = nil end}}
local quiet = {{run = function(target) end}}
local function recurse(methods, target)
    if flag then
        recurse(methods, target)
        recurse(methods, target)
    end
    methods.run(target)
end
recurse({method}, {receiver})
local observed = receiver.value
"#
        ));
        let model = ws.analysis.semantic_model(file);
        assert_eq!(
            conflict(
                &model,
                "receiver.value",
                &format!("recurse({method}, {receiver})")
            ),
            writes
        );
    }
}

#[test]
fn cached_recursive_graphs_keep_distinct_callers_separate() {
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(
        r#"
local receiver = {value = 1}
local other = {value = 1}
local methods = {run = function(target) target.value = nil end}
local function recurse(target)
    if flag then recurse(target) end
    methods.run(target)
end
recurse(other)
recurse(receiver)
local observed = receiver.value
"#,
    );
    let model = ws.analysis.semantic_model(file);
    for _ in 0..2 {
        assert!(!conflict(&model, "receiver.value", "recurse(other)"));
        assert!(conflict(&model, "receiver.value", "recurse(receiver)"));
    }
}

#[test]
fn recursive_nested_closures_retain_captured_parameters() {
    for (argument, writes) in [("receiver", true), ("other", false)] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
local receiver = {{value = 1}}
local other = {{value = 1}}
local function outer(target)
    local function inner()
        if flag then inner(); inner() end
        target.value = nil
    end
    inner()
end
outer({argument})
local observed = receiver.value
"#
        ));
        let model = ws.analysis.semantic_model(file);
        assert_eq!(
            conflict(&model, "receiver.value", &format!("outer({argument})")),
            writes
        );
    }
}

#[test]
fn cached_recursive_graphs_keep_member_replacement_positions() {
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(
        r#"
local receiver = {value = 1}
local methods = {run = function(target) target.value = nil end}
local function recurse(target)
    if flag then recurse(target) end
    methods.run(target)
end
recurse(receiver)
methods.run = function(target) end
recurse(receiver)
local observed = receiver.value
"#,
    );
    let model = ws.analysis.semantic_model(file);
    let calls: Vec<_> = model
        .chunk()
        .unwrap()
        .descendants::<LuaCallExpr>()
        .filter(|call| call.get_text() == "recurse(receiver)")
        .collect();
    assert_eq!(calls.len(), 2);
    let guarded = index(&model, "receiver.value");
    for i in [1, 0, 1, 0] {
        assert_eq!(
            calls_conflict(
                &model,
                &guarded,
                calls[i].syntax(),
                calls[i].get_range().end()
            ),
            i == 0
        );
    }
}

#[test]
fn nested_closures_retain_captured_method_receivers() {
    for (argument, writes) in [("receiver", true), ("other", false)] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
local receiver = {{value = 1}}
local other = {{value = 1}}
local methods = {{}}
function methods:outer()
    local function inner()
        if flag then inner(); inner() end
        self.value = nil
    end
    inner()
end
methods.outer({argument})
local observed = receiver.value
"#
        ));
        let model = ws.analysis.semantic_model(file);
        assert_eq!(
            conflict(
                &model,
                "receiver.value",
                &format!("methods.outer({argument})")
            ),
            writes
        );
    }
}

#[test]
fn later_caller_edges_revisit_cached_member_resolution() {
    for (member, writes) in [("value", true), ("other", false)] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
local receiver = {{value = 1}}
local methods = {{run = function(target) end}}
local function invoke(target) methods.run(target) end
local function early(target) invoke(target) end
local function late(target)
    methods.run = function(value) value.{member} = nil end
    invoke(target)
end
local function delayed(target) late(target) end
local function outer(target)
    delayed(target)
    early(target)
end
outer(receiver)
local observed = receiver.value
"#
        ));
        let model = ws.analysis.semantic_model(file);
        assert_eq!(
            conflict(&model, "receiver.value", "outer(receiver)"),
            writes
        );
    }
}
