use super::*;
use crate::VirtualWorkspace;

#[test]
fn visible_callee_chains_preserve_receiver_identity() {
    for (argument, conflicts) in [("receiver", true), ("other", false)] {
        let mut ws = VirtualWorkspace::new();
        let source = format!(
            r#"
local receiver = {{value = 1}}
local other = {{value = 1}}
local function leaf(target) target.value = nil end
local function middle(target) leaf(target) end
local function outer(target) middle(target) end
outer({argument})
local observed = receiver.value
"#
        );
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == format!("outer({argument})"))
            .unwrap();
        assert_eq!(
            calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
            conflicts
        );
    }
}
#[test]
fn called_member_function_definitions_are_visible_writes() {
    for (receiver, conflicts) in [("receiver", true), ("other", false)] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!("local receiver = {{value = 1}}\nlocal other = {{}}\nlocal function replace() function {receiver}:value() end end\nreplace()\nlocal observed = receiver.value"));
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == "replace()")
            .unwrap();
        assert_eq!(
            calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
            conflicts
        );
    }
}

#[test]
fn repeated_member_queries_through_visible_callees() {
    let mut source =
        "local receiver = {value = 1}\nlocal function step0(target) return target.value end\n"
            .to_string();
    for step in 1..64 {
        source.push_str(&format!(
            "local function step{step}(target) return step{}(target) end\n",
            step - 1
        ));
    }
    source.push_str("step63(receiver)\n");
    for query in 0..128 {
        source.push_str(&format!("local observed{query} = receiver.value\n"));
    }
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(&source);
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let call = chunk
        .descendants::<LuaCallExpr>()
        .find(|call| call.get_text() == "step63(receiver)")
        .unwrap();
    let reads = chunk
        .descendants::<LuaIndexExpr>()
        .filter(|index| index.get_text() == "receiver.value")
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 128);
    for read in reads {
        assert!(!calls_conflict(
            &model,
            &read,
            call.syntax(),
            call.get_range().end()
        ));
    }
}

#[test]
fn mutable_captured_member_callees_without_invocation_are_opaque() {
    for (initial, replacement, conflicts) in [
        (
            "target.value = nil",
            "methods.clear = function() end",
            false,
        ),
        (
            "",
            "function methods.clear(target) target.value = nil end",
            false,
        ),
        ("target.value = nil", "", true),
    ] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!("local methods = {{}}\nfunction methods.clear(target) {initial} end\nlocal function check(receiver)\nmethods.clear(receiver)\nlocal observed = receiver.value\nend\n{replacement}\ncheck({{}})"));
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == "methods.clear(receiver)")
            .unwrap();
        assert_eq!(
            calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
            conflicts,
            "{initial}; {replacement}"
        );
    }
}

#[test]
fn member_callee_presence_cache_separates_file_views() {
    let mut ws = VirtualWorkspace::new();
    let harmless = ws.def("local receiver = {value = 1}\nlocal function invoke() end\ninvoke()\nlocal value = receiver.value");
    let writer = ws.def("local receiver = {value = 1}\nlocal methods = {}\nfunction methods.invoke(target) target.value = nil end\nmethods.invoke(receiver)\nlocal value = receiver.value");
    let base = ws.analysis.semantic_model(harmless);
    for (file, call_text, expected) in [
        (harmless, "invoke()", false),
        (writer, "methods.invoke(receiver)", true),
        (harmless, "invoke()", false),
    ] {
        let model = base.model_for(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == call_text)
            .unwrap();
        assert_eq!(
            calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
            expected
        );
    }
}

#[test]
fn invocation_fingerprints_preserve_semantic_equality() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def("local function invoke(target) end");
    let model = ws.analysis.semantic_model(file);
    let closure = model
        .chunk()
        .unwrap()
        .descendants::<LuaClosureExpr>()
        .next()
        .unwrap();
    let id = SemanticId::decl(
        file,
        closure
            .get_params_list()
            .unwrap()
            .get_params()
            .next()
            .unwrap()
            .get_range(),
    );
    let paths = vec![(id.clone(), vec![(id.clone(), Vec::new())])];
    let left = Invocation::new(
        LuaClosureId::from_node(closure.syntax()),
        closure.get_syntax_id(),
        &paths,
    );
    let right = Invocation::new(left.closure, left.site, &paths);
    assert!(!Rc::ptr_eq(&left.bindings, &right.bindings));
    assert!(left == right);
    let hash = |entry: &Invocation| {
        let mut state = DefaultHasher::new();
        entry.hash(&mut state);
        state.finish()
    };
    assert_eq!(hash(&left), hash(&right));
    let mut collision = Invocation::new(left.closure, left.site, &vec![(id, Vec::new())]);
    collision.bindings_hash = left.bindings_hash;
    assert_eq!(hash(&left), hash(&collision));
    assert!(left != collision);
}

fn assert_known_call_conflicts(definitions: &str, call_text: &str) {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(&format!("local receiver = {{value = 1}}\n{definitions}\n{call_text}\nlocal observed = receiver.value"));
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let index = chunk
        .descendants::<LuaIndexExpr>()
        .find(|index| index.get_text() == "receiver.value")
        .unwrap();
    let call = chunk
        .descendants::<LuaCallExpr>()
        .find(|call| call.get_text() == call_text)
        .unwrap();
    assert!(calls_conflict(
        &model,
        &index,
        call.syntax(),
        call.get_range().end()
    ));
}

// Keep stress cases identifiable without disabling their regression coverage.
mod stress;

#[test]
fn long_callee_aliases_retain_visible_writes() {
    let mut definitions = "local function alias0(target) target.value = nil end\n".to_string();
    for alias in 1..40 {
        definitions.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
    }
    assert_known_call_conflicts(&definitions, "alias39(receiver)");
}

#[test]
fn distant_callee_rebinding_retains_visible_writes() {
    let mut definitions = "local function writer(target) target.value = nil end\nlocal action = function() end\naction = writer\nlocal count = 0\n".to_string();
    for _ in 0..300 {
        definitions.push_str("count = count + 1\n");
    }
    assert_known_call_conflicts(&definitions, "action(receiver)");
}

#[test]
fn long_formal_aliases_retain_visible_writes() {
    let mut definitions =
        "local other = {}\nlocal function outer(target)\nlocal alias0 = target\ntarget = other\n"
            .to_string();
    for alias in 1..40 {
        definitions.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
    }
    definitions.push_str("alias39.value = nil\nend");
    assert_known_call_conflicts(&definitions, "outer(receiver)");
}

#[test]
fn distant_formal_rebinding_retains_visible_writes() {
    let mut definitions =
        "local other = {}\nlocal function outer(target)\ntarget = receiver\nlocal count = 0\n"
            .to_string();
    for _ in 0..300 {
        definitions.push_str("count = count + 1\n");
    }
    definitions.push_str("target.value = nil\nend");
    assert_known_call_conflicts(&definitions, "outer(other)");
}

#[test]
fn detached_member_captures_do_not_target_the_replacement() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(
            "local receiver = {child = {value = 1}}\nlocal saved = receiver.child\nreceiver.child = {value = 2}\nlocal function write(target) target.value = nil end\nwrite(saved)\nlocal observed = receiver.child.value",
        );
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let index = chunk
        .descendants::<LuaIndexExpr>()
        .find(|index| index.get_text() == "receiver.child.value")
        .unwrap();
    let call = chunk
        .descendants::<LuaCallExpr>()
        .find(|call| call.get_text() == "write(saved)")
        .unwrap();
    assert!(!calls_conflict(
        &model,
        &index,
        call.syntax(),
        call.get_range().end()
    ));
}
