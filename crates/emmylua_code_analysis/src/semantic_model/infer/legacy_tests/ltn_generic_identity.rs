use crate::{DiagnosticCode, VirtualWorkspace};

#[test]
fn ltn_generic_copy_after_member_guard() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def("---@generic T\n---@param object T\n---@return T\nfunction table.deepcopy(object) end");
    assert!(ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, r#"
---@class flib_data_util
local flib_data_util = {}
---@generic T : table<string, any>
---@param prototype T?
---@param new_name string
---@return T
function flib_data_util.copy_prototype(prototype, new_name)
    if not (prototype and prototype.type and prototype.name) then
        error("Invalid prototype")
    end
    ---@type T
    local p = table.deepcopy(prototype)
    p.name = new_name
    return p
end
"#));
}

#[test]
fn ltn_generic_trace() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr, LuaCallExpr};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def("---@generic T\n---@param object T\n---@return T\nfunction table.deepcopy(object) end");
    let file = ws.def(r#"
---@class flib_data_util
local flib_data_util = {}
---@generic T : table<string, any>
---@param prototype T?
---@param new_name string
---@return T
function flib_data_util.copy_prototype(prototype, new_name)
    if not (prototype and prototype.type and prototype.name) then
        error("Invalid prototype")
    end
    ---@type T
    local p = table.deepcopy(prototype)
    p.name = new_name
    return p
end
"#);
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    for name in chunk.descendants::<LuaNameExpr>() {
        if name.get_name_text().as_deref() == Some("prototype") {
            eprintln!("PROTOTYPE {:?} raw={:?} flow={:?}", name.get_position(), model.type_of_expr(name.get_syntax_id()), model.type_of_expr_at(name.get_syntax_id(), name.get_position()));
        }
    }
    for call in chunk.descendants::<LuaCallExpr>() {
        eprintln!("CALL {:?} raw={:?} flow={:?}", call.get_position(), model.type_of_expr(call.get_syntax_id()), model.type_of_expr_at(call.get_syntax_id(), call.get_position()));
    }
}
