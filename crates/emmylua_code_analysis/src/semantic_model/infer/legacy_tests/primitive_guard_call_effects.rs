//! Visible call effects grouped by evaluation order, receiver identity and recursion.

mod aliases;
mod call_boundaries;
mod member_callees;
mod rawset;
mod recursion;

use crate::{DiagnosticCode, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaIndexExpr};

fn assert_primitive_call_boundary(body: &str, nullable: bool) {
    assert_member_call_boundary("type(elem.handler) == 'table'", "", body, nullable);
}

fn assert_member_call_boundary(guard: &str, setup: &str, body: &str, nullable: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let mut config = ws.get_emmyrc();
    config.runtime.nonstandard_symbol = serde_json::from_str(r#"["continue"]"#).unwrap();
    ws.update_emmyrc(config);
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::NeedCheckNil);
    let source = format!(
        r#"
---@class CallBoundaryHandler
---@field name string
local Handler = {{}}
---@class CallBoundaryReceiver
---@field handler CallBoundaryHandler?
local Receiver = {{}}
---@param elem CallBoundaryReceiver
---@param key string
---@param flag boolean
---@param other CallBoundaryReceiver
local function dispatch(elem, key, flag, other)
    local function clear() if flag then elem[key] = nil end end
{setup}
    if {guard} then
{body}
    end
end
"#
    );
    let marker = source.find("-- observe").unwrap();
    let start = source[..marker].rfind('\n').unwrap() + 1;
    let expected = ws.ty(if nullable {
        "CallBoundaryHandler?"
    } else {
        "CallBoundaryHandler"
    });
    let file = ws.def(&source);
    let model = ws.analysis.semantic_model(file);
    let field = model
        .chunk()
        .unwrap()
        .descendants::<LuaIndexExpr>()
        .filter(|field| {
            field.get_text() == "elem.handler"
                && (start..marker).contains(&(u32::from(field.get_position()) as usize))
        })
        .max_by_key(LuaAstNode::get_position)
        .unwrap();
    let actual = model.type_of_expr_at(field.get_syntax_id(), field.get_position());
    assert_eq!(actual, expected, "{body}");
    assert_eq!(
        model.type_of_expr_at(field.get_syntax_id(), field.get_position()),
        expected,
        "warm query: {body}"
    );
    let diagnostics = ws
        .analysis
        .diagnose_file(file, tokio_util::sync::CancellationToken::new())
        .unwrap();
    let line = source[..marker].bytes().filter(|ch| *ch == b'\n').count() as u32;
    assert_eq!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == line),
        nullable,
        "{body}: {diagnostics:#?}"
    );
}
