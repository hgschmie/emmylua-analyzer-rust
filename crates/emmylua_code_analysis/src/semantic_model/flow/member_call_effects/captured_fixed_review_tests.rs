use super::*;
#[test]
fn fixed_captured_member_callees_work_without_an_outer_invocation() {
    for setup in [
        "local methods={clear=function() receiver.value=nil end}",
        "local methods={}; function methods.clear() receiver.value=nil end",
        "local methods={}; methods.clear=function() receiver.value=nil end",
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        let source = format!(
            "local receiver={{value=1}}; {setup}; local function dispatch() methods.clear(); local observed=receiver.value end"
        );
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .filter(|index| index.get_text() == "receiver.value")
            .last()
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == "methods.clear()")
            .unwrap();
        assert!(
            calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
            "{setup}"
        );
    }
}
