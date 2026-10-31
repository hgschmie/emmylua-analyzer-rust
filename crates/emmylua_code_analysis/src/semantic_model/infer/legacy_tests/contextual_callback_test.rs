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
    let chunk = model.chunk().expect("test file has a syntax tree");
    let mut locals = chunk.descendants::<LuaLocalName>().filter(|local| {
        local
            .get_name_token()
            .is_some_and(|token| token.get_name_text() == name)
    });
    let local = locals.next().expect("observed local exists in the fixture");
    assert!(
        locals.next().is_none(),
        "observed local {name} must be unique"
    );
    let decl = model
        .decl_by_offset(local.get_name_token().unwrap().get_position())
        .expect("observed local resolves to a declaration");
    model
        .type_of_decl(&decl)
        .expect("observed local has a type")
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

#[gtest]
fn contextual_callback_optional_owner_contracts() {
    for (owner, key, accepted) in [
        ("table<string,Entry>?", "name", true),
        ("Entry[]?", "[1]", true),
        ("table<string,Entry?>", "name", true),
        ("(Entry?)[]", "[1]", true),
        ("table<string,Entry>|unknown", "name", false),
        ("table<string,Entry>|boolean", "name", false),
        ("table<string,Entry>|table<integer,Entry>", "name", false),
        ("nil", "name", false),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(&format!(
            r#"
            ---@type {owner}
            local entries = {{ {key} = {{ callback = function(settings)
                local observed = settings
            end }} }}
        "#
        ));
        assert_eq!(local_type(&ws, file, "entries"), ws.ty(owner), "{owner}");
        if owner == "(Entry?)[]" {
            assert!(
                matches!(local_type(&ws, file, "entries"), LuaType::Array(ref array) if array.get_base().is_nullable())
            );
        }
        let expected = if accepted {
            ws.ty("Settings")
        } else {
            LuaType::Unknown
        };
        expect_that!(local_type(&ws, file, "observed"), eq(&expected), "{owner}");
    }
}

#[gtest]
fn contextual_callback_optional_member_owner_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    ws.def(
        r#"
        ---@class Entry
        ---@field callback SettingFunction
        ---@class Container
        ---@field entries table<string,Entry>?
    "#,
    );
    let file = ws.def(
        r#"
        ---@type Container
        local container = {}
        container.entries = { name = { callback = function(settings)
            local observed = settings
        end } }
    "#,
    );
    expect_that!(local_type(&ws, file, "observed"), eq(&ws.ty("Settings")));
}

#[gtest]
fn contextual_callback_explicit_local_and_assignment_params() {
    for (doc, expected) in [
        ("settings boolean", "boolean"),
        ("settings? boolean", "boolean?"),
        ("settings unknown", "unknown"),
        ("settings any", "any"),
        ("settings T", "T"),
    ] {
        for (setup, target) in [
            ("---@type SettingFunction\n", "local handler"),
            (
                "---@type table<string,SettingFunction>\nlocal handlers = {}\n",
                "handlers.foo",
            ),
        ] {
            let mut ws = VirtualWorkspace::new();
            ws.def(DEFINITIONS);
            let file = ws.def(&format!(
                r#"
                {setup}---@generic T: string
                ---@param {doc}
                {target} = function(settings)
                    local observed = settings
                end
            "#
            ));
            let actual = local_type(&ws, file, "observed");
            let model = ws.analysis.semantic_model(file);
            let closure = model
                .chunk()
                .unwrap()
                .descendants::<emmylua_parser::LuaClosureExpr>()
                .next()
                .unwrap();
            let signature = model.type_of_signature(closure.get_syntax_id()).unwrap();
            let signature_param = signature.get_params()[0].1.as_ref().unwrap();
            expect_that!(
                signature_param,
                eq(&actual),
                "signature/body: {doc}, {target}"
            );
            if expected == "T" {
                expect_true!(
                    matches!(actual, LuaType::TplRef(ref template)
                    if template.get_name() == "T" && template.get_constraint() == Some(&LuaType::String)),
                    "{doc}, {target}: {actual:?}"
                );
            } else {
                expect_that!(actual, eq(&ws.ty(expected)), "{doc}, {target}");
            }
        }
    }
}

#[gtest]
fn contextual_callback_unannotated_nested_member_chain_terminates() {
    for prefix in [
        "local container = {}; container.entries = ",
        "local container = { nested = {} }; container.nested.entries = ",
        "local entries = ",
    ] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
            {prefix}{{ name = {{ callback = function(settings)
                local observed = settings
            end }} }}
        "#
        ));
        let ty = local_type(&ws, file, "observed");
        assert!(
            matches!(ty, LuaType::Unknown | LuaType::Any),
            "{prefix}: {ty:?}"
        );
    }
}

#[gtest]
fn contextual_callback_parenthesized_constructors_keep_context() {
    for (prefix, value) in [
        (
            "---@type table<string,SettingFunction>\nlocal handlers = ",
            "({ callback = function(settings) local observed = settings end })",
        ),
        (
            "---@type table<string,SettingFunction>\nlocal handlers\nhandlers = ",
            "(({ callback = function(settings) local observed = settings end }))",
        ),
        (
            "---@class Container\n---@field handlers table<string,SettingFunction>\n---@type Container\nlocal container\ncontainer.handlers = ",
            "({ callback = function(settings) local observed = settings end })",
        ),
        (
            "---@type table<string,Entry>\nlocal entries = ",
            "{ name = ({ callback = function(settings) local observed = settings end }) }",
        ),
        (
            "local entries = ",
            "{\n---@type Entry\nname = ({ callback = function(settings) local observed = settings end }) }",
        ),
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(&format!("{prefix}{value}"));
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&ws.ty("Settings")),
            "{prefix}{value}"
        );
    }
}

#[gtest]
fn contextual_callback_field_generic_parameters_keep_scope() {
    for (doc, nullable) in [("settings T", false), ("settings? T", true)] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        let file = ws.def(&format!(
            r#"
            ---@type table<string,SettingFunction>
            local handlers = {{
                ---@generic T: string
                ---@param {doc}
                handler = function(settings)
                    local observed = settings
                end,
            }}
        "#
        ));
        let actual = local_type(&ws, file, "observed");
        let alternatives = match &actual {
            LuaType::Union(union) => union.into_vec(),
            ty => vec![ty.clone()],
        };
        expect_true!(
            alternatives
                .iter()
                .any(|ty| matches!(ty, LuaType::TplRef(template)
            if template.get_name() == "T" && template.get_constraint() == Some(&LuaType::String))),
            "{doc}: {actual:?}"
        );
        expect_that!(actual.is_nullable(), eq(nullable), "{doc}");
        let model = ws.analysis.semantic_model(file);
        let closure = model
            .chunk()
            .unwrap()
            .descendants::<emmylua_parser::LuaClosureExpr>()
            .next()
            .unwrap();
        let signature = model.type_of_signature(closure.get_syntax_id()).unwrap();
        expect_that!(
            signature.get_params()[0].1.as_ref().unwrap(),
            eq(&actual),
            "signature/body: {doc}"
        );
        expect_that!(signature.get_generic_params().len(), eq(1), "{doc}");
        let cross_file_signature = model
            .type_of_signature_in_file(file, closure.get_syntax_id())
            .unwrap();
        expect_that!(
            cross_file_signature.get_params()[0].1.as_ref().unwrap(),
            eq(&actual),
            "cross-file signature/body: {doc}"
        );
        expect_that!(
            cross_file_signature.get_generic_params(),
            eq(signature.get_generic_params()),
            "{doc}"
        );
    }
}

#[gtest]
fn contextual_callback_array_contract_accepts_integer_keys() {
    for key in ["0", "-1", "1"] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(&format!(
            r#"
            ---@type Entry[]
            local entries = {{ [{key}] = {{ callback = function(settings)
                local observed = settings
            end }} }}
        "#
        ));
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&ws.ty("Settings")),
            "key={key}"
        );
    }
}

#[gtest]
fn contextual_callback_parentheses_do_not_invent_context() {
    for source in [
        "local entries = ({ name = ({ callback = function(settings) local observed = settings end }) })",
        "---@type table<integer,Entry>\nlocal entries = ({ wrong = ({ callback = function(settings) local observed = settings end }) })",
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def("---@class Entry\n---@field callback SettingFunction");
        let file = ws.def(source);
        let actual = local_type(&ws, file, "observed");
        expect_true!(
            matches!(actual, LuaType::Unknown | LuaType::Any),
            "{source}: {actual:?}"
        );
    }
}

#[gtest]
fn contextual_callback_field_generics_do_not_leak_between_siblings() {
    let mut ws = VirtualWorkspace::new();
    ws.def("---@alias T boolean");
    let file = ws.def(
        r#"
        ---@type table<string, fun(value: unknown): unknown>
        local handlers = {
            ---@generic T: string
            ---@param value T
            ---@return T
            text = function(value)
                local observed_text = value
                return value
            end,
            ---@generic T: integer
            ---@param value T
            ---@return T
            number = function(value)
                local observed_number = value
                return value
            end,
            plain = function(value)
                local observed_plain = value
                return value
            end,
        }
    "#,
    );
    let model = ws.analysis.semantic_model(file);
    let closures = model
        .chunk()
        .unwrap()
        .descendants::<emmylua_parser::LuaClosureExpr>()
        .collect::<Vec<_>>();
    for (index, (name, constraint)) in [
        ("observed_text", LuaType::String),
        ("observed_number", LuaType::Integer),
    ]
    .into_iter()
    .enumerate()
    {
        let actual = local_type(&ws, file, name);
        expect_true!(
            matches!(&actual, LuaType::TplRef(template) if template.get_name() == "T" && template.get_constraint() == Some(&constraint)),
            "{name}: {actual:?}"
        );
        for signature in [
            model
                .type_of_signature(closures[index].get_syntax_id())
                .unwrap(),
            model
                .type_of_signature_in_file(file, closures[index].get_syntax_id())
                .unwrap(),
        ] {
            expect_that!(signature.get_generic_params().len(), eq(1), "{name}");
            expect_that!(
                signature.get_params()[0].1.as_ref().unwrap(),
                eq(&actual),
                "{name}"
            );
            let generic = &signature.get_generic_params()[0];
            expect_that!(generic.get_constraint(), eq(Some(&constraint)), "{name}");
            expect_true!(
                matches!(signature.get_ret(), LuaType::TplRef(template)
                if template.get_name() == generic.get_name() && template.get_tpl_id() == generic.get_tpl_id()),
                "{name}"
            );
        }
    }
    expect_that!(
        local_type(&ws, file, "observed_plain"),
        eq(&LuaType::Unknown)
    );
    expect_true!(
        model
            .type_of_signature(closures[2].get_syntax_id())
            .unwrap()
            .get_generic_params()
            .is_empty()
    );
}

#[gtest]
fn contextual_callback_named_unknown_and_nil_contracts_stay_authoritative() {
    for contract in ["unknown", "nil"] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def(&format!(
            r#"
            ---@class Entry
            ---@field callback SettingFunction
            ---@class Container
            ---@field selected {contract}
            ---@field [string] Entry
        "#
        ));
        let file = ws.def(
            r#"
            ---@type Container
            local entries = { selected = { callback = function(settings)
                local observed = settings
            end } }
        "#,
        );
        let actual = local_type(&ws, file, "observed");
        expect_true!(
            matches!(actual, LuaType::Unknown | LuaType::Any),
            "{contract}: {actual:?}"
        );
    }
}

#[gtest]
fn contextual_callback_nested_unsigned_keys_use_checked_integer_domain() {
    use crate::EmmyrcLuaVersion;
    use emmylua_parser::{LuaIndexKey, LuaTableField, NumberResult};
    for owner in ["table<integer,Entry>", "Entry[]"] {
        for value in [0_u64, 1, i64::MAX as u64, i64::MAX as u64 + 1, u64::MAX] {
            let mut ws = VirtualWorkspace::new();
            let mut config = ws.get_emmyrc();
            config.runtime.version = EmmyrcLuaVersion::LuaJIT;
            ws.update_emmyrc(config);
            ws.def(DEFINITIONS);
            ws.def("---@class Entry\n---@field callback SettingFunction");
            let file = ws.def(&format!(
                r#"
                ---@type {owner}
                local entries = {{ [{value}ULL] = {{ callback = function(settings)
                    local observed = settings
                end }} }}
            "#
            ));
            let model = ws.analysis.semantic_model(file);
            let parsed = model
                .chunk()
                .unwrap()
                .descendants::<LuaTableField>()
                .find_map(|field| match field.get_field_key()? {
                    LuaIndexKey::Integer(token) => Some(token.get_number_value()),
                    _ => None,
                })
                .unwrap();
            let correct_classification = match parsed {
                NumberResult::Int(actual) => u64::try_from(actual) == Ok(value),
                NumberResult::Uint(actual) => actual == value && value > i64::MAX as u64,
                _ => false,
            };
            assert!(correct_classification, "{value}: {parsed:?}");
            let expected = if i64::try_from(value).is_ok() {
                ws.ty("Settings")
            } else {
                LuaType::Unknown
            };
            expect_that!(
                local_type(&ws, file, "observed"),
                eq(&expected),
                "{owner}: {value}ULL"
            );
        }
    }
}

#[gtest]
fn contextual_callback_field_generics_survive_unrelated_signature_docs() {
    for outer_docs in ["", "---@deprecated\n", "---@async\n"] {
        for optional in ["", "?"] {
            let mut ws = VirtualWorkspace::new();
            ws.def(DEFINITIONS);
            let file = ws.def(&format!(
                r#"
                {outer_docs}---@type table<string,SettingFunction>
                local handlers = {{
                    ---@generic T: string
                    ---@param settings{optional} T
                    handler = function(settings)
                        local observed = settings
                    end,
                }}
            "#
            ));
            let foreign_file = ws.def("local unrelated = true");
            let model = ws.analysis.semantic_model(file);
            let foreign_model = ws.analysis.semantic_model(foreign_file);
            let closure = model
                .chunk()
                .unwrap()
                .descendants::<emmylua_parser::LuaClosureExpr>()
                .next()
                .unwrap();
            let facts = model
                .file_facts()
                .unwrap()
                .signature_by_closure(closure.get_syntax_id())
                .unwrap();
            expect_that!(
                facts.docs.is_some(),
                eq(!outer_docs.is_empty()),
                "outer={outer_docs:?}"
            );
            let actual = local_type(&ws, file, "observed");
            let alternatives = match &actual {
                LuaType::Union(union) => union.into_vec(),
                ty => vec![ty.clone()],
            };
            expect_true!(alternatives.iter().any(|ty| matches!(ty, LuaType::TplRef(template)
                if template.get_name() == "T" && template.get_constraint() == Some(&LuaType::String))), "{outer_docs:?}, {optional:?}: {actual:?}");
            expect_that!(actual.is_nullable(), eq(!optional.is_empty()));
            for signature in [
                model.type_of_signature(closure.get_syntax_id()).unwrap(),
                model
                    .type_of_signature_in_file(file, closure.get_syntax_id())
                    .unwrap(),
                foreign_model
                    .type_of_signature_in_file(file, closure.get_syntax_id())
                    .unwrap(),
            ] {
                expect_that!(
                    signature.get_params()[0].1.as_ref().unwrap(),
                    eq(&actual),
                    "{outer_docs:?}, {optional:?}"
                );
                expect_that!(
                    signature.get_generic_params().len(),
                    eq(1),
                    "{outer_docs:?}, {optional:?}"
                );
            }
        }
    }
}

#[gtest]
fn contextual_callback_equivalent_owner_alternatives_share_context() {
    for annotation in [
        "table<string,Entry>|table<string,Entry>",
        "FirstMap|SecondMap",
        "(FirstMap|SecondMap)?",
    ] {
        let mut ws = VirtualWorkspace::new();
        ws.def(DEFINITIONS);
        ws.def(
            r#"
            ---@class Entry
            ---@field callback SettingFunction
            ---@alias FirstMap table<string,Entry>
            ---@alias SecondMap table<string,Entry>
        "#,
        );
        let file = ws.def(&format!(
            r#"
            ---@type {annotation}
            local entries = {{ named = {{ callback = function(settings)
                local observed = settings
            end }} }}
        "#
        ));
        expect_that!(
            local_type(&ws, file, "observed"),
            eq(&ws.ty("Settings")),
            "{annotation}"
        );
    }
}

#[gtest]
fn contextual_callback_unrelated_docs_do_not_import_ancestor_field_contracts() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(
        r#"
        local handlers = {
            ---@generic T: string
            ---@param settings T
            ---@return T
            handler = function(settings)
                local nested = {}
                ---@deprecated
                nested.callback = function(settings)
                    return settings
                end
                return settings
            end,
        }
    "#,
    );
    let foreign_file = ws.def("local unrelated = true");
    let model = ws.analysis.semantic_model(file);
    let foreign_model = ws.analysis.semantic_model(foreign_file);
    let closure = model
        .chunk()
        .unwrap()
        .descendants::<emmylua_parser::LuaClosureExpr>()
        .nth(1)
        .unwrap();
    let docs = model
        .file_facts()
        .unwrap()
        .signature_by_closure(closure.get_syntax_id())
        .unwrap()
        .docs
        .as_ref()
        .unwrap();
    assert!(docs.deprecated && docs.param_types.is_empty() && docs.returns.is_empty());
    for signature in [
        model
            .type_of_signature_in_file(file, closure.get_syntax_id())
            .unwrap(),
        foreign_model
            .type_of_signature_in_file(file, closure.get_syntax_id())
            .unwrap(),
    ] {
        expect_that!(
            signature.get_params()[0].1.as_ref().unwrap(),
            eq(&LuaType::Any)
        );
        expect_true!(signature.get_generic_params().is_empty());
        expect_false!(matches!(
            signature.get_ret(),
            LuaType::TplRef(_) | LuaType::Ref(_)
        ));
    }
}

#[gtest]
fn contextual_callback_collected_generic_contracts_remain_authoritative() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFINITIONS);
    let file = ws.def(
        r#"
        ---@generic T: integer
        ---@param settings T
        ---@return T
        ---@type table<string,SettingFunction>
        local handlers = {
            ---@generic U: string
            ---@param settings U
            ---@return U
            handler = function(settings)
                return settings
            end,
        }
    "#,
    );
    let foreign_file = ws.def("local unrelated = true");
    let model = ws.analysis.semantic_model(file);
    let foreign_model = ws.analysis.semantic_model(foreign_file);
    let closure = model
        .chunk()
        .unwrap()
        .descendants::<emmylua_parser::LuaClosureExpr>()
        .next()
        .unwrap();
    for signature in [
        model.type_of_signature(closure.get_syntax_id()).unwrap(),
        model
            .type_of_signature_in_file(file, closure.get_syntax_id())
            .unwrap(),
        foreign_model
            .type_of_signature_in_file(file, closure.get_syntax_id())
            .unwrap(),
    ] {
        assert_eq!(signature.get_generic_params().len(), 1);
        let generic = &signature.get_generic_params()[0];
        expect_that!(generic.get_name(), eq("T"));
        expect_that!(generic.get_constraint(), eq(Some(&LuaType::Integer)));
        for ty in [
            signature.get_params()[0].1.as_ref().unwrap(),
            signature.get_ret(),
        ] {
            expect_true!(
                matches!(ty, LuaType::TplRef(template)
                if template.get_name() == "T" && template.get_tpl_id() == generic.get_tpl_id()),
                "{ty:?}"
            );
        }
    }
}
