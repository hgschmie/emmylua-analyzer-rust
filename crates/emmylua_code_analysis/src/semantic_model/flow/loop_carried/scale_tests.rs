//! Construction and independent query behavior in files containing many loops.
use crate::{LuaType, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaNameExpr};

#[test]
fn many_independent_loops_keep_separate_entry_values() {
    let mut source = String::new();
    for index in 0..256 {
        let (initial, update) = if index % 2 == 0 {
            ("0", "value+1")
        } else {
            ("''", "value..'x'")
        };
        source.push_str(&format!("local function f_{index}() local value={initial}; for i=1,3 do local observed=value; value={update} end end\n"));
    }
    let mut ws = VirtualWorkspace::new();
    let start = std::time::Instant::now();
    let file = ws.def(&source);
    let build_elapsed = start.elapsed();
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    for (marker, expected) in [
        (source.find("local observed=value"), LuaType::Integer),
        (source.rfind("local observed=value"), LuaType::String),
    ] {
        let offset = marker.unwrap() + "local observed=".len();
        let value = chunk
            .descendants::<LuaNameExpr>()
            .find(|name| usize::from(name.get_position()) == offset)
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(value.get_syntax_id(), value.get_position()),
            expected
        );
    }
    println!(
        "256 independent loops: source_bytes={}, flow_nodes={}, build={build_elapsed:?}",
        source.len(),
        model.flow_tree().unwrap().node_count()
    );
}
