#![cfg(test)]

use googletest::prelude::*;

use crate::{DiagnosticCode, Emmyrc, LuaType, VirtualWorkspace};

fn workspace(table_index: bool) -> VirtualWorkspace {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let mut config = ws.get_emmyrc();
    config.strict.table_index = table_index;
    ws.update_emmyrc(config);
    ws
}

#[gtest]
fn table_index_is_opt_in() {
    assert_that!(Emmyrc::default().strict.table_index, eq(false));
    let config: Emmyrc = serde_json::from_str(r#"{"strict":{}}"#).unwrap();
    expect_that!(config.strict.table_index, eq(false));
    let config: Emmyrc = serde_json::from_str(r#"{"strict":{"tableIndex":true}}"#).unwrap();
    expect_that!(config.strict.table_index, eq(true));
    expect_that!(
        serde_json::to_value(config).unwrap()["strict"]["tableIndex"],
        eq(true)
    );

    for enabled in [false, true] {
        for annotation in ["table<string, string>", "{[string]: string}"] {
            let mut ws = workspace(enabled);
            ws.def(&format!("---@type {annotation}\nmods = {{}}"));
            let expected = ws.ty(if enabled { "string?" } else { "string" });
            expect_that!(ws.expr_ty("mods['nullius']"), eq(&expected));
            expect_that!(ws.expr_ty("mods.nullius"), eq(&expected));
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::UnnecessaryIf,
                    "if mods['nullius'] then end\nif not mods['nullius'] then end"
                ),
                eq(enabled)
            );
        }
    }
}

#[gtest]
fn table_index_supports_key_and_value_types() {
    let mut ws = workspace(true);
    ws.def(
        r#"
        ---@type table<integer, boolean>
        numeric = {}
        ---@type {[boolean]: number}
        boolean_key = {}
        ---@type table<string, any>
        any_values = {}
        ---@type {[string]: unknown}
        unknown_values = {}
        ---@type table<string, string?>
        optional_values = {}
        ---@type string
        key = ''
        ---@type table<string, string>
        strings = {}
    "#,
    );
    let boolean_optional = ws.ty("boolean?");
    expect_that!(ws.expr_ty("numeric[42]"), eq(&boolean_optional));
    let number_optional = ws.ty("number?");
    expect_that!(ws.expr_ty("boolean_key[true]"), eq(&number_optional));
    expect_that!(ws.expr_ty("any_values.missing"), eq(&LuaType::Any));
    expect_that!(ws.expr_ty("unknown_values.missing"), eq(&LuaType::Unknown));
    let string_optional = ws.ty("string?");
    expect_that!(ws.expr_ty("optional_values.missing"), eq(&string_optional));
    expect_that!(ws.expr_ty("strings[key]"), eq(&string_optional));
}

#[gtest]
fn table_index_preserves_required_fields_and_function_returns() {
    let mut ws = workspace(true);
    ws.def(
        r#"
        ---@type {required: string, [string]: string}
        record = {}
        ---@class RequiredRecord
        ---@field required string
        ---@type RequiredRecord
        named = {}
        function make_lookup()
            return setmetatable({}, {
                ---@param key string
                ---@return string
                __index = function(_, key) return key end
            })
        end
        fallback = make_lookup()
        ---@type table<string, string>
        local values = {}
        table_fallback = setmetatable({}, { __index = values })
    "#,
    );
    expect_that!(ws.expr_ty("record.required"), eq(&LuaType::String));
    expect_that!(ws.expr_ty("named.required"), eq(&LuaType::String));
    expect_that!(ws.expr_ty("fallback.missing"), eq(&LuaType::String));
    let string_optional = ws.ty("string?");
    expect_that!(ws.expr_ty("record.missing"), eq(&string_optional));
    expect_that!(ws.expr_ty("table_fallback.missing"), eq(&string_optional));
    expect_that!(
        ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, "if record.required then end"),
        eq(false)
    );
}

#[gtest]
fn table_index_follows_aliases_generics_and_nested_dictionaries() {
    let mut ws = workspace(true);
    ws.def(
        r#"
        ---@alias Dictionary<T> table<string, T>
        ---@alias ObjectDictionary<T> {[string]: T}
        ---@type Dictionary<string>
        dictionary = {}
        ---@type ObjectDictionary<string>
        object_dictionary = {}
        ---@type Dictionary<Dictionary<string>>
        nested = {}
        if nested.entry then
            nested_value = nested.entry.value
        end
        ---@type table<string, string>|{[string]: number}
        alternatives = {}
    "#,
    );
    let string_optional = ws.ty("string?");
    expect_that!(ws.expr_ty("dictionary.entry"), eq(&string_optional));
    expect_that!(ws.expr_ty("object_dictionary.entry"), eq(&string_optional));
    expect_that!(ws.expr_ty("nested_value"), eq(&string_optional));
    let union = ws.ty("string|number|nil");
    expect_that!(ws.expr_ty("alternatives.entry"), eq(&union));
}

#[gtest]
fn table_index_guards_and_assignments_narrow_reads() {
    for annotation in ["table<string, string>", "{[string]: string}"] {
        let mut ws = workspace(true);
        ws.def(&format!(
            r#"
            ---@type {annotation}
            local map = {{}}
            if map.entry then
                guarded = map.entry
            end
            local value = map.other
            if value then
                guarded_local = value
            end
            ---@param input string
            function update(input)
                map.entry = input
                after_write = map.entry
                map.entry = nil
                after_delete = map.entry
            end
        "#
        ));
        expect_that!(ws.expr_ty("guarded"), eq(&LuaType::String));
        expect_that!(ws.expr_ty("guarded_local"), eq(&LuaType::String));
        expect_that!(ws.expr_ty("after_write"), eq(&LuaType::String));
        expect_that!(ws.expr_ty("after_delete"), eq(&LuaType::Nil));
    }
}

#[gtest]
fn table_index_requires_a_guard_before_dereferencing() {
    let mut ws = workspace(true);
    ws.def("---@type table<string, {name: string}>\nrecords = {}");
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            "local name = records.missing.name"
        ),
        eq(false)
    );
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            "if records.present then local name = records.present.name end"
        ),
        eq(true)
    );
}

#[gtest]
fn table_index_preserves_writes_and_nil_deletion() {
    for enabled in [false, true] {
        for annotation in ["table<string, string>", "{[string]: string}"] {
            let mut ws = workspace(enabled);
            ws.def(&format!("---@type {annotation}\nmap = {{}}"));
            for source in [
                "map.entry = 'ok'",
                "map.entry = nil",
                "map.entry = map.entry or 'ok'",
            ] {
                expect_that!(
                    ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
                    eq(true),
                    "{source}"
                );
            }
            expect_that!(
                ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, "map.entry = 42"),
                eq(false)
            );
        }
    }
}

#[gtest]
fn table_index_preserves_contextual_types_in_initializers() {
    let mut ws = workspace(true);
    ws.def(
        r#"
        ---@class Entry
        ---@field name string
        ---@field callback fun(value: string)
        ---@type table<string, Entry>
        entries = {}
        entries.example = {
            name = 'ok',
            callback = function(value) contextual_value = value end,
        }
    "#,
    );
    expect_that!(ws.expr_ty("contextual_value"), eq(&LuaType::String));
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            "entries.example = { name = 'ok', callback = function(value) end }"
        ),
        eq(true)
    );
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            "entries.example = { name = 42, callback = function(value) end }"
        ),
        eq(false)
    );
}

#[gtest]
fn table_index_does_not_make_iteration_values_nullable() {
    for annotation in ["table<string, string>", "{[string]: string}"] {
        let mut ws = workspace(true);
        ws.def(&format!(
            r#"
            ---@type {annotation}
            local map = {{}}
            for key, value in pairs(map) do
                iter_key = key
                iter_value = value
            end
            next_key, next_value = next(map)
        "#
        ));
        expect_that!(ws.expr_ty("iter_key"), eq(&LuaType::String));
        expect_that!(ws.expr_ty("iter_value"), eq(&LuaType::String));
        let optional = ws.ty("string?");
        expect_that!(ws.expr_ty("next_key"), eq(&optional));
        expect_that!(ws.expr_ty("next_value"), eq(&optional));
    }
}

#[gtest]
fn table_index_is_independent_of_array_index() {
    for enabled in [false, true] {
        let mut ws = workspace(enabled);
        let mut config = ws.get_emmyrc();
        config.strict.array_index = !enabled;
        ws.update_emmyrc(config);
        ws.def("---@type string[]\narray = {}\n---@type table<integer, string>\nmap = {}");
        let array_expected = ws.ty(if enabled { "string" } else { "string?" });
        let map_expected = ws.ty(if enabled { "string?" } else { "string" });
        expect_that!(ws.expr_ty("array[123]"), eq(&array_expected));
        expect_that!(ws.expr_ty("map[123]"), eq(&map_expected));
    }
}

#[gtest]
fn table_index_accepts_nullable_dictionary_writes() {
    for enabled in [false, true] {
        for annotation in [
            "table<string, uint32>",
            "{[string]: uint32}",
            "Dictionary<uint32>",
        ] {
            let mut ws = workspace(enabled);
            let source = format!(
                r#"
                ---@alias uint32 integer
                ---@alias Dictionary<T> table<string, T>
                ---@param target {annotation}
                ---@param key string
                ---@param value uint32?
                ---@param positive boolean
                local function update(target, key, value, positive)
                    target[key] = value
                    target[key] = positive and value or nil
                    target[key] = nil
                end
            "#
            );
            expect_that!(
                ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
                eq(true),
                "strict.tableIndex={enabled}, {annotation}"
            );
        }
    }
}

#[gtest]
fn table_index_nullable_writes_still_check_values_and_required_fields() {
    for enabled in [false, true] {
        for (annotation, target, value_type) in [
            ("table<string, integer>", "target[key]", "string?"),
            ("{[string]: integer}", "target[key]", "string?"),
            ("Dictionary<integer>", "target[key]", "boolean?"),
            (
                "{required: integer, [string]: integer}",
                "target.required",
                "integer?",
            ),
            ("Required", "target.required", "integer?"),
            ("Directional", "target.required", "integer?"),
        ] {
            let mut ws = workspace(enabled);
            let source = format!(
                r#"
                ---@alias Dictionary<T> table<string, T>
                ---@class Required
                ---@field required integer
                ---@class Directional
                ---@field (read) required integer?
                ---@field (write) required integer
                ---@type {annotation}
                target = {{}}
                ---@type string
                key = ''
                ---@type {value_type}
                value = nil
            "#
            );
            ws.def(&source);
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::AssignTypeMismatch,
                    &format!("{target} = value")
                ),
                eq(false),
                "strict.tableIndex={enabled}, {annotation}, {target}, {value_type}"
            );
        }
    }
}
