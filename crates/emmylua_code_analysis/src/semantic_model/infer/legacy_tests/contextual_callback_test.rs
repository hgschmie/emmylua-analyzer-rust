use crate::{DiagnosticCode, FileId, LuaType, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaAstToken, LuaLocalName};
use googletest::prelude::*;

const DEFINITIONS: &str = r#"
---@class Settings
---@field count integer
---@alias SettingFunction fun(settings: Settings, name: string)
---@alias Callback<T> fun(value: T, name: string)
---@alias HandlerMap<T> table<string, Callback<T>>
---@alias CallbackArray<T> Callback<T>[]
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
        "SettingFunction[]",
        "Callback<Settings>[]",
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

#[gtest]
fn contextual_callback_array_parameters() {
    for (annotation, entry) in [
        ("SettingFunction[]", ""),
        ("Callback<Settings>[]", "[2] = "),
        ("CallbackArray<Settings>", ""),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        let file = ws.def(&format!(
            r#"
            ---@type {annotation}
            local handlers = {{ {entry}function(settings, name)
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
fn contextual_callback_annotated_member_array_parameters() {
    let mut ws = VirtualWorkspace::new();
    ws.def(r#"
        ---@class TrainInfo
        ---@field train_name string
        ---@class Player
        ---@field index integer
        ---@alias DropDownFunction fun(train_info: TrainInfo, entity_type: string?, player: Player?): string?
    "#);
    let file = ws.def(
        r#"
        local Constants = {}
        ---@type DropDownFunction[]
        Constants.filter_dropdown_values = {
            function(train_info) return train_info.train_name end,
            function(train_info, entity_type, player)
                local observed_train = train_info
                local observed_entity = entity_type
                local observed_player = player
                return nil
            end,
        }
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_train"),
        eq(&ws.ty("TrainInfo"))
    );
    expect_that!(
        local_type(&ws, file, "observed_entity"),
        eq(&ws.ty("string?"))
    );
    expect_that!(
        local_type(&ws, file, "observed_player"),
        eq(&ws.ty("Player?"))
    );
}

#[gtest]
fn contextual_callback_array_explicit_parameters_override_context() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    let file = ws.def(
        r#"
        ---@type SettingFunction[]
        local handlers = {
            ---@param settings boolean
            ---@param name integer
            function(settings, name)
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
fn contextual_callback_unannotated_member_array_stays_untyped() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(
        r#"
        local Constants = {}
        Constants.handlers = { function(value)
            local observed_value = value
        end }
    "#,
    );
    // The work branch's broader assignment-context fallback already uses Any
    // for unannotated table members. Do not invent a callback contract.
    expect_that!(local_type(&ws, file, "observed_value"), eq(&LuaType::Any));
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

#[gtest]
fn contextual_callback_nested_constructor_keys() {
    for (annotation, entry) in [
        ("table<string, Entry>", "[Keys.name] = "),
        ("table<string, Entry>", "['name'] = "),
        ("Entry[]", ""),
        ("Entry[]", "[2] = "),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction?");
        ws.def("Keys = { name = 'name' }");
        let file = ws.def(&format!(
            r#"
            local Container = {{}}
            ---@type {annotation}
            Container.entries = {{ {entry}{{ callback = function(settings, name)
                local observed_settings = settings
                local observed_name = name
            end }} }}
        "#
        ));
        expect_that!(
            local_type(&ws, file, "observed_settings"),
            eq(&ws.ty("Settings")),
            "{annotation}/{entry}"
        );
        expect_that!(
            local_type(&ws, file, "observed_name"),
            eq(&LuaType::String),
            "{annotation}/{entry}"
        );
    }
}

#[gtest]
fn contextual_callback_nested_invalid_key_stays_unknown() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    ws.def("---@class Entry\n---@field callback SettingFunction?");
    let file = ws.def(
        r#"
        ---@type table<integer, Entry>
        local entries = { wrong = { callback = function(settings)
            local observed_settings = settings
        end } }
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_settings"),
        eq(&LuaType::Unknown)
    );
}
