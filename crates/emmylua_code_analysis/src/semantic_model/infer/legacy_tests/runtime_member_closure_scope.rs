use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaIndexExpr};

use crate::{LuaType, VirtualWorkspace};

const TYPES: &str = r#"
---@class ProbeStorage
---@field ConnectedSurfaces table<string,table<string,integer>>
---@field Other integer[]

---@type ProbeStorage
storage = {ConnectedSurfaces={}, Other={}}
"#;

fn assert_field(source: &str, same_file: bool, expect_runtime_value: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let expected = ws.ty("table<string,table<string,integer>>");
    let types = if expect_runtime_value {
        TYPES.replace(
            "ConnectedSurfaces table<string,table<string,integer>>",
            "ConnectedSurfaces unknown",
        )
    } else {
        TYPES.to_owned()
    };
    let source = if same_file {
        format!(
            "{}\n{source}",
            types.replace(
                "storage = {ConnectedSurfaces={}, Other={}}",
                "storage = make_storage()"
            )
        )
    } else {
        ws.def_file("types.lua", &types);
        source.to_owned()
    };
    let marker = source.find("-- observe").unwrap();
    let line_start = source[..marker].rfind('\n').map_or(0, |pos| pos + 1);
    let file = ws.def_file("case.lua", &source);
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let field = chunk
        .descendants::<LuaIndexExpr>()
        .find(|expr| {
            let pos = u32::from(expr.get_position()) as usize;
            expr.get_text().ends_with(".ConnectedSurfaces") && pos >= line_start && pos < marker
        })
        .unwrap();
    let actual = model.type_of_expr_at(field.get_syntax_id(), field.get_position());
    if expect_runtime_value {
        let value = chunk
            .descendants::<LuaAssignStat>()
            .filter_map(|assignment| {
                let (targets, values) = assignment.get_var_and_expr_list();
                targets
                    .first()?
                    .get_text()
                    .ends_with(".ConnectedSurfaces")
                    .then(|| values.first().unwrap().clone())
            })
            .last()
            .unwrap();
        let runtime = model.type_of_expr_at(value.get_syntax_id(), value.get_position());
        assert!(matches!(runtime, LuaType::TableConst(_)), "{runtime:?}");
        assert_eq!(actual, runtime);
    } else {
        assert_eq!(actual, expected);
    }
}

#[test]
fn runtime_member_scope_ignores_uncalled_sibling_closure() {
    assert_field(
        r#"
local function clear()
    storage.ConnectedSurfaces = {}
end

---@param key string
---@param entity_key string
local function connect(key, entity_key)
    storage.ConnectedSurfaces[key] = storage.ConnectedSurfaces[key] or {}
    local surface_connections = storage.ConnectedSurfaces[key] -- observe
    surface_connections[entity_key] = 1
end
return connect
"#,
        false,
        false,
    );
}

#[test]
fn runtime_member_scope_ignores_uncalled_nested_closure() {
    assert_field(
        r#"
local function connect()
    local function clear() storage.ConnectedSurfaces = {} end
    local observed = storage.ConnectedSurfaces -- observe
end
"#,
        false,
        false,
    );
}

#[test]
fn runtime_member_scope_preserves_no_reset_contract() {
    assert_field(
        "local function connect()\nlocal observed = storage.ConnectedSurfaces -- observe\nend",
        false,
        false,
    );
}

#[test]
fn runtime_member_scope_preserves_other_field_contract() {
    assert_field(
        r#"
local function clear() storage.Other = {} end
local function connect()
    local observed = storage.ConnectedSurfaces -- observe
end
"#,
        false,
        false,
    );
}

#[test]
fn runtime_member_scope_ignores_later_closure() {
    assert_field(
        r#"
local function connect()
    local observed = storage.ConnectedSurfaces -- observe
end
local function clear() storage.ConnectedSurfaces = {} end
"#,
        false,
        false,
    );
}

#[test]
fn runtime_member_scope_preserves_same_file_declared_contract() {
    assert_field(
        r#"
local function clear() storage.ConnectedSurfaces = {} end
local function connect()
    local observed = storage.ConnectedSurfaces -- observe
end
"#,
        true,
        false,
    );
}

#[test]
fn runtime_member_scope_preserves_same_closure_write() {
    assert_field(
        r#"
local function connect()
    storage.ConnectedSurfaces = {}
    local observed = storage.ConnectedSurfaces -- observe
end
"#,
        false,
        true,
    );
}

#[test]
fn runtime_member_scope_preserves_top_level_write() {
    assert_field(
        r#"
storage.ConnectedSurfaces = {}
local function connect()
    local observed = storage.ConnectedSurfaces -- observe
end
"#,
        false,
        true,
    );
}

#[test]
fn runtime_member_scope_preserves_enclosing_closure_write() {
    assert_field(
        r#"
local function make()
    storage.ConnectedSurfaces = {}
    local function connect()
        local observed = storage.ConnectedSurfaces -- observe
    end
    return connect
end
"#,
        false,
        true,
    );
}

#[test]
fn runtime_member_scope_skips_sibling_before_visible_write() {
    assert_field(
        r#"
local function clear() storage.ConnectedSurfaces = {} end
local function connect()
    storage.ConnectedSurfaces = {}
    local observed = storage.ConnectedSurfaces -- observe
end
"#,
        false,
        true,
    );
}

#[test]
fn runtime_member_scope_preserves_implicit_self_contract() {
    assert_field(
        r#"
function storage:connect()
    self.ConnectedSurfaces = {}
    local observed = self.ConnectedSurfaces -- observe
end
"#,
        false,
        true,
    );
}

#[test]
fn runtime_member_scope_preserves_explicit_receiver_contract() {
    assert_field(
        r#"
---@param receiver ProbeStorage
local function connect(receiver)
    receiver.ConnectedSurfaces = {}
    local observed = receiver.ConnectedSurfaces -- observe
end
"#,
        false,
        true,
    );
}

#[test]
fn runtime_member_scope_function_value_uses_enclosing_scope() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let expected = ws.ty("fun():string");
    ws.def_file(
        "types.lua",
        r#"
---@class CallbackOwner
---@field callback unknown

---@type CallbackOwner
callbacks = {}
"#,
    );
    let file = ws.def_file(
        "case.lua",
        r#"
callbacks.callback = function() return 'hello' end
local observed = callbacks.callback
"#,
    );
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let read = chunk.descendants::<LuaIndexExpr>().last().unwrap();
    let assignment = chunk.descendants::<LuaAssignStat>().next().unwrap();
    let (_, values) = assignment.get_var_and_expr_list();
    assert_eq!(
        model.type_of_expr_at(values[0].get_syntax_id(), values[0].get_position()),
        expected
    );
    let actual = model.type_of_expr_at(read.get_syntax_id(), read.get_position());
    assert!(actual.is_function(), "{actual:?}");
}
