use crate::{DiagnosticCode, FileId, LuaType, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaAstToken, LuaLocalName};
use googletest::prelude::*;

const DEFINITIONS: &str = r#"
---@class Settings
---@field count integer
---@alias SettingFunction fun(settings: Settings, name: string)
---@alias Callback<T> fun(value: T, name: string)
---@alias HandlerMap<T> table<string, Callback<T>>
"#;

#[gtest]
fn contextual_callback_dictionary_parameters() {
    for (annotation, key) in [
        ("table<string, SettingFunction>", "['setting-name']"),
        (
            "table<string, fun(settings: Settings, name: string)>",
            "named",
        ),
        ("table<integer, SettingFunction>", "[1]"),
        ("HandlerMap<Settings>", "named"),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        let file = ws.def(&format!(
            r#"
            ---@type {annotation}
            local handlers = {{ {key} = function(settings, name)
                local observed_settings = settings
                local observed_name = name
            end }}
        "#
        ));
        expect_that!(
            local_type(&ws, file, "observed_settings"),
            eq(&ws.ty("Settings")),
            "{annotation}"
        );
        expect_that!(
            local_type(&ws, file, "observed_name"),
            eq(&LuaType::String),
            "{annotation}"
        );
    }
}

#[gtest]
fn contextual_callback_decl_and_member_assignment_parameters() {
    for prefix in [
        "---@type SettingFunction\nlocal handler = ",
        "---@type table<string, SettingFunction>\nlocal handlers = {}\nhandlers.foo = ",
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        let file = ws.def(&format!(
            r#"{prefix}function(settings, name)
            local observed_settings = settings
            local observed_name = name
        end"#
        ));
        expect_that!(
            local_type(&ws, file, "observed_settings"),
            eq(&ws.ty("Settings")),
            "{prefix}"
        );
        expect_that!(
            local_type(&ws, file, "observed_name"),
            eq(&LuaType::String),
            "{prefix}"
        );
    }
}

#[gtest]
fn contextual_callback_body_checks_use_declared_parameters() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    for (code, body) in [
        (
            DiagnosticCode::AssignTypeMismatch,
            "settings.count = 'wrong'",
        ),
        (DiagnosticCode::ParamTypeMismatch, "accept_integer(name)"),
    ] {
        expect_false!(ws.has_no_diagnostic(
            code,
            &format!(
                r#"
            ---@param value integer
            local function accept_integer(value) end
            ---@type table<string, SettingFunction>
            local handlers = {{ foo = function(settings, name) {body} end }}
        "#
            )
        ));
    }
}

#[gtest]
fn contextual_callback_explicit_parameters_override_expected_type() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    let file = ws.def(
        r#"
        ---@type table<string, SettingFunction>
        local handlers = {
            ---@param settings boolean
            ---@param name integer
            foo = function(settings, name)
                local observed_settings = settings
                local observed_name = name
            end,
        }
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_settings"),
        eq(&LuaType::Boolean)
    );
    expect_that!(
        local_type(&ws, file, "observed_name"),
        eq(&LuaType::Integer)
    );
}

#[gtest]
fn contextual_callback_incompatible_key_does_not_supply_parameters() {
    for annotation in [
        "table<integer, SettingFunction>",
        "table<string, integer>",
        "table",
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        let file = ws.def(&format!(
            r#"
            ---@type {annotation}
            local handlers = {{ wrong = function(settings)
                local observed_settings = settings
            end }}
        "#
        ));
        expect_that!(
            local_type(&ws, file, "observed_settings"),
            eq(&LuaType::Unknown),
            "{annotation}"
        );
    }
}

#[gtest]
fn contextual_callback_alias_cycles_terminate() {
    let mut ws = VirtualWorkspace::new();
    ws.def("---@alias CycleA CycleB\n---@alias CycleB CycleA");
    let file = ws.def(
        r#"
        ---@type table<string, CycleA>
        local handlers = { foo = function(settings)
            local observed_settings = settings
        end }
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_settings"),
        eq(&LuaType::Unknown)
    );
}

fn local_type(ws: &VirtualWorkspace, file: FileId, name: &str) -> LuaType {
    let model = ws.analysis.semantic_model(file);
    let local = model
        .chunk()
        .unwrap()
        .descendants::<LuaLocalName>()
        .find(|local| {
            local
                .get_name_token()
                .is_some_and(|token| token.get_name_text() == name)
        })
        .unwrap();
    let decl = model
        .decl_by_offset(local.get_name_token().unwrap().get_position())
        .unwrap();
    model.type_of_decl(&decl).unwrap()
}
