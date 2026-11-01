//! Numeric loop reachability at nested loop-entry transfers.
use crate::{LuaType, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaNameExpr};

fn observed_type(ws: &mut VirtualWorkspace, source: &str) -> LuaType {
    let file = ws.def(source);
    let offset = source.find("local observed=value").unwrap() + "local observed=".len();
    let model = ws.analysis.semantic_model(file);
    let name = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| usize::from(name.get_position()) == offset)
        .unwrap();
    model.type_of_expr_at(name.get_syntax_id(), name.get_position())
}

#[test]
fn zero_iteration_numeric_loops_do_not_widen_outer_values() {
    for body in [
        "for i=1,0 do value='text' end; local observed=value",
        "for outer=1,3 do for inner=1,0 do value='text' end; local observed=value end",
        "for outer=1,3 do for inner=0,1,-1 do value='text' end; local observed=value end",
        "for outer=1,3 do for inner=0.5,0.25,0.1 do value='text' end; local observed=value end",
        "for outer=1,3 do for inner=9007199254740995,9007199254740994 do value='text' end; local observed=value end",
        "for outer=1,3 do for inner=(-2),(-1),(-1) do function value() end end; local observed=value end",
    ] {
        let mut ws = VirtualWorkspace::new();
        let source = format!("local value=0; {body}");
        assert_eq!(
            observed_type(&mut ws, &source),
            LuaType::IntegerConst(0),
            "{body}"
        );
    }
}

#[test]
fn possible_numeric_iterations_retain_their_writes() {
    for bounds in [
        "1,1",
        "0,1",
        "1,0,-1",
        "first,last",
        "1,2,0",
        "9007199254740993,9007199254740992.0,1",
    ] {
        let source = format!(
            "local value=0; for outer=1,3 do for inner={bounds} do value='text' end; local observed=value end"
        );
        let mut ws = VirtualWorkspace::new();
        let ty = observed_type(&mut ws, &source);
        let includes_write = match &ty {
            LuaType::String | LuaType::StringConst(_) => true,
            LuaType::Union(union) => union
                .into_vec()
                .iter()
                .any(|ty| matches!(ty, LuaType::String | LuaType::StringConst(_))),
            _ => false,
        };
        assert!(includes_write, "{bounds}: {ty:?}");
    }
}

#[test]
fn numeric_loop_empty_proof_respects_runtime_integer_support() {
    use crate::EmmyrcLuaVersion;
    let source = "local value=0; for outer=1,3 do local observed=value; for inner=9007199254740993,9007199254740992 do value='text' end end";
    for (version, empty) in [
        (EmmyrcLuaVersion::Lua51, false),
        (EmmyrcLuaVersion::Lua52, false),
        (EmmyrcLuaVersion::LuaJIT, false),
        (EmmyrcLuaVersion::Lua53, true),
        (EmmyrcLuaVersion::Lua54, true),
    ] {
        let mut ws = VirtualWorkspace::new();
        let mut config = ws.get_emmyrc();
        config.runtime.version = version;
        ws.update_emmyrc(config);
        let ty = observed_type(&mut ws, source);
        assert_eq!(ty == LuaType::IntegerConst(0), empty, "{version:?}: {ty:?}");
    }
}
