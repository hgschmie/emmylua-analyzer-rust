//! Declared parameter contracts across called nested-closure writes.
use crate::{DiagnosticCode, LuaType, VirtualWorkspace};
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
fn captured_invalid_write_does_not_replace_parameter_contract() {
    let source = "---@param value integer\nlocal function outer(value)\n local function mutate() value='text' end\n if cond then mutate() end\n local observed=value\nend";
    let mut ws = VirtualWorkspace::new();
    assert_eq!(observed_type(&mut ws, source), LuaType::Integer);
    assert!(ws.has_diagnostic(DiagnosticCode::AssignTypeMismatch, source));
}
