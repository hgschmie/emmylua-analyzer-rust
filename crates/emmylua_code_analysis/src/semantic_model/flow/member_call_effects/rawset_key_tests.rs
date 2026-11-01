use super::*;
use crate::VirtualWorkspace;

#[test]
fn numeric_rawset_keys_keep_receiver_and_ancestor_identity() {
    for (guarded, call_text, conflicts) in [
        ("elem[1].handler", "rawset(elem, 1.0, nil)", true),
        ("elem[1].handler", "rawset(elem, 1.5, nil)", false),
        ("elem[2].handler", "rawset(elem, 1.0, nil)", false),
        ("elem[1].handler", "rawset(other, 1.0, nil)", false),
        ("elem.handler", "rawset(elem, dynamic, nil)", true),
        ("elem.handler", "rawset(elem, mixed, nil)", true),
        ("elem.handler", "rawset(elem, numeric, nil)", false),
        ("elem[1]", "rawset(elem, numeric, nil)", true),
        ("elem.child.handler", "rawset(elem, 'child', nil)", true),
        ("elem.child.handler", "rawset(elem.child, 1.5, nil)", false),
        ("elem[1].handler", "rawset(elem[1], 'handler', nil)", true),
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(&format!("---@param numeric number\n---@param mixed number|string\nlocal function inspect(elem, other, numeric, dynamic, mixed)\n{call_text}\nlocal observed = {guarded}\nend"));
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == guarded)
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == call_text)
            .unwrap();
        assert_eq!(
            calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
            conflicts,
            "{guarded}; {call_text}"
        );
    }
}
