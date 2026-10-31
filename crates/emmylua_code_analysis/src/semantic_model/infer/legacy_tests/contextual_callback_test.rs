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
fn contextual_callback_unannotated_member_array_stays_unknown() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(
        r#"
        local Constants = {}
        Constants.handlers = { function(value)
            local observed_value = value
        end }
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_value"),
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

#[gtest]
fn contextual_callback_computed_enum_member_key() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    ws.def("---@class Entry\n---@field callback SettingFunction?");
    let file = ws.def(
        r#"
        ---@class Sorting
        ---@field keys string[]
        ---@field sorting table<string, SortColumn?>
        ---@field entries table<SortColumn, Entry>
        local Sorting = {
            ---@enum SortColumn
            keys = { 'name', 'id' },
            sorting = {},
        }
        for _, key in pairs(Sorting.keys) do
            Sorting.sorting[key] = key --[[@as SortColumn]]
        end
        Sorting.entries = {
            [Sorting.sorting.name] = { callback = function(settings, name)
                local observed_settings = settings
                local observed_name = name
            end }
        }
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_settings"),
        eq(&ws.ty("Settings"))
    );
    expect_that!(local_type(&ws, file, "observed_name"), eq(&LuaType::String));
}

#[gtest]
fn contextual_callback_nil_key_has_no_value_context() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    ws.def("---@class Entry\n---@field callback SettingFunction?");
    let file = ws.def(
        r#"
        ---@type nil
        local key
        ---@type table<string, Entry>
        local entries = { [key] = { callback = function(settings)
            local observed_settings = settings
        end } }
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_settings"),
        eq(&LuaType::Unknown)
    );
}

#[gtest]
fn contextual_callback_nullable_key_retains_value_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    ws.def("---@class Entry\n---@field callback SettingFunction?");
    let file = ws.def(
        r#"
        ---@param key string?
        local function make(key)
            ---@type table<string, Entry>
            local entries = { [key] = { callback = function(settings)
                local observed_settings = settings
            end } }
        end
    "#,
    );
    expect_that!(
        local_type(&ws, file, "observed_settings"),
        eq(&ws.ty("Settings"))
    );
}

#[gtest]
fn contextual_callback_long_plain_alias_keeps_inline_contract() {
    // Straddle the former 32-alias cap and retain longer finite chains.
    // The shared evaluator's query-depth limit is not an alias-chain limit.
    for depth in [31, 32, 64, 80] {
        let mut ws = VirtualWorkspace::new();
        let mut definitions = DEFINITIONS.to_string();
        for index in 0..depth {
            definitions.push_str(&format!("\n---@alias Chain{index} Chain{}", index + 1));
        }
        definitions.push_str(&format!("\n---@alias Chain{depth} fun(value: Settings)"));
        ws.def(&definitions);
        let file = ws.def("local handlers = {\n---@type Chain0\n handler = function(value) local observed = value end\n}");
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&ws.ty("Settings")),
            "depth={depth}"
        );
    }
}

#[gtest]
fn contextual_callback_user_table_does_not_invent_index_contract() {
    for (name, modifier, with_std) in [
        ("table", "", false),
        ("table", "(internal) ", false),
        ("table", "(internal) ", true),
        ("Table", "", true),
        ("Custom.table", "", true),
    ] {
        let mut ws = if with_std {
            VirtualWorkspace::new_with_init_std_lib()
        } else {
            VirtualWorkspace::new()
        };
        ws.def(DEFINITIONS);
        ws.def(&format!(
            "---@class {modifier}{name}<K,V>\n---@field other V\nlocal CustomTable = {{}}"
        ));
        let file = ws.def(&format!("---@type {name}<string,SettingFunction>\nlocal handlers = {{ wrong = function(settings) local observed = settings end }}"));
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&LuaType::Unknown),
            "{modifier}{name}, std={with_std}"
        );
    }
}

#[gtest]
fn contextual_callback_unknown_nested_key_has_no_value_contract() {
    use emmylua_parser::{LuaIndexKey, LuaTableField};
    for owner in ["Entry[]", "table<string, Entry>"] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(&format!("---@type {owner}\nlocal entries = {{ [missing_key] = {{ callback = function(settings) local observed = settings end }} }}"));
        let model = ws.analysis.semantic_model(file);
        let key = model
            .chunk()
            .unwrap()
            .syntax()
            .descendants()
            .filter_map(LuaTableField::cast)
            .find_map(|field| match field.get_field_key()? {
                LuaIndexKey::Expr(expr) => Some(expr),
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            model.type_of_expr(key.get_syntax_id()),
            LuaType::Unknown
        ));
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&LuaType::Unknown),
            "{owner}"
        );
    }
}

#[gtest]
fn contextual_callback_generic_aliases_share_evaluation_bounds() {
    let mut ws = VirtualWorkspace::new();
    let mut definitions = DEFINITIONS.to_string();
    for index in 0..80 {
        definitions.push_str(&format!(
            "\n---@alias GenericChain{index}<T> GenericChain{}<T>",
            index + 1
        ));
    }
    definitions.push_str("\n---@alias GenericChain80<T> fun(value:T)");
    ws.def(&definitions);
    let file = ws.def("---@type table<string,GenericChain0<Settings>>\nlocal handlers = { handler = function(value) local observed = value end }");
    expect_that!(local_type(&ws, file, "observed"), eq(&ws.ty("Settings")));

    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    ws.def("---@alias Growing<T> Growing<T[]>");
    let file = ws.def("---@type table<string,Growing<Settings>>\nlocal handlers = { handler = function(value) local observed = value end }");
    expect_that!(local_type(&ws, file, "observed"), eq(&LuaType::Unknown));
}

#[gtest]
fn contextual_callback_builtin_table_contract_is_preserved() {
    for with_std in [false, true] {
        for (domain, valid) in [("string", true), ("integer", false)] {
            let mut ws = if with_std {
                VirtualWorkspace::new_with_init_std_lib()
            } else {
                VirtualWorkspace::new()
            };
            ws.def(DEFINITIONS);
            let file = ws.def(&format!("---@type table<{domain},SettingFunction>\nlocal handlers = {{ handler = function(settings) local observed = settings end }}"));
            let expected = if valid {
                ws.ty("Settings")
            } else {
                LuaType::Unknown
            };
            expect_that!(
                local_type(&ws, file, "observed"),
                eq(&expected),
                "{domain}, std={with_std}"
            );
        }
    }
}

#[gtest]
fn contextual_callback_index_key_contract_boundaries() {
    for (setup, expected) in [
        ("---@type table<string,string?>\nlocal keys", true),
        ("---@type table<integer,string>\nlocal keys", false),
        ("local keys = {}", false),
        ("local keys = { name = nil }", false),
        ("local keys = { name = false }", false),
        ("local keys = { name = 'name' }", true),
        (
            "---@class table<K,V>\n---@field other V\nlocal CustomTable = {}\n---@type table<string,string>\nlocal keys",
            false,
        ),
        (
            "---@class (internal) table<K,V>\n---@field other V\nlocal CustomTable = {}\n---@type table<string,string>\nlocal keys",
            false,
        ),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(&format!("{setup}\n---@type table<string,Entry>\nlocal entries = {{ [keys.name] = {{ callback = function(settings) local observed = settings end }} }}"));
        let expected = if expected {
            ws.ty("Settings")
        } else {
            LuaType::Unknown
        };
        expect_that!(local_type(&ws, file, "observed"), eq(&expected), "{setup}");
    }
}

#[gtest]
fn contextual_callback_unknown_union_key_has_no_value_contract() {
    use emmylua_parser::{LuaIndexKey, LuaTableField};
    for annotation in ["unknown?", "unknown|string", "MissingKey"] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction\n---@alias MissingKey\n---| unknown\n---| nil");
        let file = ws.def(&format!(
            r#"
            ---@type {annotation}
            local index
            ---@type table<string, Entry>
            local entries = {{ [index] = {{ callback = function(settings)
                local observed = settings
            end }} }}
        "#
        ));
        let model = ws.analysis.semantic_model(file);
        let key = model
            .chunk()
            .unwrap()
            .descendants::<LuaTableField>()
            .find_map(|field| match field.get_field_key()? {
                LuaIndexKey::Expr(expr) => Some(expr),
                _ => None,
            })
            .unwrap();
        let actual = model.type_of_expr_at(key.get_syntax_id(), key.get_position());
        let expanded = match actual {
            LuaType::Union(union) => LuaType::Union(union),
            LuaType::Ref(id) if annotation == "MissingKey" => model
                .alias_target(&model.type_def_of(&id).unwrap())
                .unwrap(),
            other => panic!("expected a composite or alias key, got {other:?}"),
        };
        assert!(
            matches!(expanded, LuaType::Union(ref union) if union.iter().any(LuaType::is_unknown)),
            "{annotation}: {expanded:?}"
        );
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&LuaType::Unknown),
            "{annotation}"
        );
    }
}

#[gtest]
fn contextual_callback_unsigned_index_does_not_borrow_negative_contract() {
    use crate::EmmyrcLuaVersion;
    use emmylua_parser::{LuaIndexExpr, LuaIndexKey, NumberResult};
    for (literal, unsigned, value) in [
        ("18446744073709551615ULL", true, -1),
        ("0xffffffffffffffff", false, -1),
        ("3000000000", false, 3_000_000_000),
    ] {
        let mut ws = VirtualWorkspace::new();
        let mut config = ws.get_emmyrc();
        config.runtime.version = EmmyrcLuaVersion::LuaJIT;
        ws.update_emmyrc(config);
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(&format!(
            r#"
            ---@type table<{value},string>
            local names
            ---@type table<string,Entry>
            local entries = {{ [names[{literal}]] = {{ callback = function(settings)
                local observed = settings
            end }} }}
        "#
        ));
        let model = ws.analysis.semantic_model(file);
        let key = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find_map(|index| match index.get_index_key()? {
                LuaIndexKey::Integer(value) => Some(value.get_number_value()),
                _ => None,
            })
            .unwrap();
        assert!(
            if unsigned {
                matches!(key, NumberResult::Uint(u64::MAX))
            } else {
                matches!(key, NumberResult::Int(actual) if actual == value)
            },
            "{literal}: {key:?}"
        );
        let expected = if unsigned {
            LuaType::Unknown
        } else {
            ws.ty("Settings")
        };
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&expected),
            "{literal}"
        );
    }
}

#[gtest]
fn contextual_callback_known_key_domains_survive_normalization() {
    for (key_type, owner, accepted) in [
        ("string?", "table<string,Entry>", true),
        ("integer?", "Entry[]", true),
        ("any", "table<string,Entry>", true),
        ("any?", "table<string,Entry>", true),
        ("boolean?", "table<string,Entry>", false),
        ("{value: unknown}", "table<{value: unknown},Entry>", true),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(&format!(
            r#"
            ---@type {key_type}
            local key
            ---@type {owner}
            local entries = {{ [key] = {{ callback = function(settings)
                local observed = settings
            end }} }}
        "#
        ));
        let expected = if accepted {
            ws.ty("Settings")
        } else {
            LuaType::Unknown
        };
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&expected),
            "{key_type}, {owner}"
        );
    }
}

#[gtest]
fn contextual_callback_assignment_uses_direct_table_position() {
    for (targets, prefix) in [("first, entries, last", "many(), "), ("entries, last", "")] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        let file = ws.def(&format!(
            r#"
            ---@return integer, integer
            local function many() return 1, 2 end
            ---@type table<string,SettingFunction>
            local entries
            local first, last
            {targets} = {prefix}{{ callback = function(settings)
                local observed = settings
            end }}
        "#
        ));
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&ws.ty("Settings")),
            "{targets}"
        );
    }
}

#[gtest]
fn contextual_callback_aliases_only_use_callable_alternatives() {
    for (annotation, expected) in [
        ("DataAlias", LuaType::Any),
        (
            "CallbackAlias|DataAlias|fun(value: boolean)",
            LuaType::from_vec(vec![LuaType::Integer, LuaType::Boolean]),
        ),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class Data
            ---@field value string
            ---@alias DataAlias Data
            ---@alias CallbackAlias fun(value: integer)
        "#,
        );
        let file = ws.def(&format!(
            r#"
            local handlers = {{
                ---@type {annotation}
                callback = function(value)
                    local observed = value
                end,
            }}
        "#
        ));
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&expected),
            "{annotation}"
        );
    }
}
