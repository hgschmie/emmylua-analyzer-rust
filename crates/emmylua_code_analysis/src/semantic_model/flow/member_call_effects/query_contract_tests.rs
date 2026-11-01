use super::*;

#[test]
fn repeated_call_effect_queries_scan_receiver_paths_once_per_file() {
    let mut ws = crate::VirtualWorkspace::new();
    let source = "local receiver = {value=1}\nlocal other = {value=1}\nlocal methods = {}\nfunction methods.clear(target) target.value = nil end\nmethods.clear(receiver)\nlocal observed = receiver.value\nlocal unrelated = other.value";
    let a = ws.def_file("receiver-a.lua", source);
    let b = ws.def_file("receiver-b.lua", source);
    let root_model = ws.analysis.semantic_model(a);
    RECEIVER_TARGET_SCANS.with(|count| count.set(0));
    for file in [a, b, a] {
        let model = root_model.model_for(file);
        let chunk = model.chunk().unwrap();
        let call = chunk.descendants::<LuaCallExpr>().next().unwrap();
        for (target, conflicts) in [("receiver.value", true), ("other.value", false)] {
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == target)
                .unwrap();
            for _ in 0..16 {
                assert_eq!(
                    calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
                    conflicts
                );
            }
        }
    }
    assert_eq!(
        RECEIVER_TARGET_SCANS.with(|count| count.get()),
        2,
        "snapshot: one receiver-target scan per completed model query"
    );
}

#[test]
fn primitive_guard_without_standard_type_binding_is_opaque() {
    let mut ws = crate::VirtualWorkspace::new();
    let file =
        ws.def("local value = {}\nif type(value) == 'table' then local observed = value end");
    let model = ws.analysis.semantic_model(file);
    let condition = model
        .chunk()
        .unwrap()
        .descendants::<emmylua_parser::LuaBinaryExpr>()
        .next()
        .unwrap();
    assert!(
        super::super::primitive_guard_target(&model, &LuaExpr::BinaryExpr(condition), true)
            .is_none()
    );
}

#[test]
fn statically_unreachable_callee_assignments_do_not_invalidate_guards() {
    for condition in ["false", "nil", "not true", "not 1", "not ''"] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!("local receiver = {{value=1}}\nlocal action = function() end\nif {condition} then action = function(target) target.value = nil end end\naction(receiver)\nlocal observed = receiver.value"));
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.value")
            .unwrap();
        let call = chunk.descendants::<LuaCallExpr>().next().unwrap();
        assert!(
            !calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
            "{condition}"
        );
    }
}

#[test]
fn primitive_guard_target_accepts_raw_negated_conditions() {
    use emmylua_parser::LuaIfStat;
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def("if not (type(receiver.value) ~= 'number') then end");
    let model = ws.analysis.semantic_model(file);
    let condition = model
        .chunk()
        .unwrap()
        .descendants::<LuaIfStat>()
        .next()
        .unwrap()
        .get_condition_expr()
        .unwrap();
    let (_, primitive, keep_matching) =
        super::super::primitive_guard_target(&model, &condition, true)
            .expect("raw negation and parentheses must normalize at the helper boundary");
    assert_eq!(primitive, LuaType::Number);
    assert!(keep_matching);
}

#[test]
fn member_access_path_retains_an_uninitialized_name_root() {
    use emmylua_parser::LuaNameExpr;
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local receiver; local observed = receiver");
    let model = ws.analysis.semantic_model(file);
    let name = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .next()
        .unwrap();
    let decl = model.resolve_name(name.get_position()).unwrap();
    assert_eq!(
        member_access_path(&model, &LuaExpr::NameExpr(name)),
        Some((decl, vec![]))
    );
}

#[test]
fn receiver_targets_share_fixed_paths_across_distinct_guarded_members() {
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local first={};local second={};local a=first.value;local b=second.value");
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let paths = chunk
        .descendants::<LuaIndexExpr>()
        .map(|index| member_access_path(&model, &LuaExpr::IndexExpr(index)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 2);
    let first = receiver_targets(&model, &paths[0]);
    let second = receiver_targets(&model, &paths[1]);
    assert!(
        Rc::ptr_eq(&first, &second),
        "fixed guarded paths share the cached vector without cloning its contents"
    );
}
