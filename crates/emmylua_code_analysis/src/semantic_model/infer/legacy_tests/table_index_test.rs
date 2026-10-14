#![cfg(test)]

use googletest::prelude::*;

use crate::{DiagnosticCode, Emmyrc, LuaType, VirtualWorkspace};

#[gtest]
fn table_index_handles_optional_and_fully_matching_union_keys() {
    for enabled in [false, true] {
        for annotation in ["table<integer, Entry>", "{[integer]: Entry}"] {
            let mut ws = workspace(enabled);
            ws.def(&format!(
                r#"
                ---@class Entry
                ---@field name string
                ---@type {annotation}
                entries = {{}}
                ---@type integer?
                optional_key = nil
                ---@class Entity
                ---@field (read) unit_number? integer
                ---@type Entity
                entity = {{}}
                ---@type 1|2
                valid_keys = 1
                ---@type {{[string]: string, [integer]: number}}
                multiple_signatures = {{}}
                ---@type string|integer
                signature_keys = 1
                ---@type table<string|integer, boolean>
                union_dictionary = {{}}
                ---@type {{name: string, count: number}}
                required_fields = {{}}
                ---@type 'name'|'count'
                field_keys = 'name'
            "#
            ));
            let optional = ws.ty("Entry?");
            expect_that!(ws.expr_ty("entries[optional_key]"), eq(&optional));
            expect_that!(ws.expr_ty("entries[entity.unit_number]"), eq(&optional));
            let definite = ws.ty(if enabled { "Entry?" } else { "Entry" });
            expect_that!(ws.expr_ty("entries[assert(optional_key)]"), eq(&definite));
            expect_that!(ws.expr_ty("entries[valid_keys]"), eq(&definite));
            let signatures = ws.ty(if enabled {
                "string|number|nil"
            } else {
                "string|number"
            });
            expect_that!(
                ws.expr_ty("multiple_signatures[signature_keys]"),
                eq(&signatures)
            );
            let union_value = ws.ty(if enabled { "boolean?" } else { "boolean" });
            expect_that!(
                ws.expr_ty("union_dictionary[signature_keys]"),
                eq(&union_value)
            );
            let required = ws.ty("string|number");
            expect_that!(ws.expr_ty("required_fields[field_keys]"), eq(&required));
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::UnnecessaryIf,
                    "local entry = entries[optional_key]\nif entry then end"
                ),
                eq(true)
            );
        }
    }
}

#[gtest]
fn table_index_nullable_writes_combine_with_integer_alias_arithmetic() {
    for enabled in [false, true] {
        let mut ws = workspace(enabled);
        expect_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::AssignTypeMismatch,
                r#"
                ---@alias uint32 integer
                ---@class LoadingElement
                ---@field count uint32
                ---@class Dispatcher
                ---@field Provided table<string, uint32>
                ---@field Pending table<string, uint32>
                ---@param dispatcher Dispatcher
                ---@param loading LoadingElement
                ---@param key string
                ---@param use_stacks boolean
                local function update(dispatcher, loading, key, use_stacks)
                    local provided = dispatcher.Provided
                    provided[key] = assert(provided[key]) - loading.count
                    local stacks = math.floor(assert(provided[key]) / 10)
                    if (use_stacks and stacks > 0) or
                        (not use_stacks and assert(provided[key]) > 0) then
                        dispatcher.Pending[key] = provided[key]
                    end
                    local amount = dispatcher.Pending[key]
                    if amount then
                        amount = amount - loading.count
                        dispatcher.Pending[key] = (amount > 0) and amount or nil
                    end
                end
            "#
            ),
            eq(true),
            "strict.tableIndex={enabled}"
        );
    }
}

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
        fallback = setmetatable({}, {
            ---@param key string
            ---@return string
            __index = function(_, key) return key end
        })
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
        ] {
            let mut ws = workspace(enabled);
            let source = format!(
                r#"
                ---@alias Dictionary<T> table<string, T>
                ---@class Required
                ---@field required integer
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

#[gtest]
fn table_index_computed_assignment_preserves_group_type() {
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};

    for table_index in [false, true] {
        for array_index in [false, true] {
            let mut ws = workspace(table_index);
            let mut config = ws.get_emmyrc();
            config.strict.array_index = array_index;
            ws.update_emmyrc(config);
            let source = r#"
                ---@class PriorityGroup
                ---@field free_trains integer[]
                ---@param key string
                local function add(key)
                    ---@type table<string, PriorityGroup>
                    local groups = {}
                    groups[key] = groups[key] or { free_trains = {} }
                    local trains = groups[key].free_trains
                    local group = groups[key]
                end
            "#;
            assert!(ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, source));
            let file = ws.def(source);
            let model = ws.analysis.semantic_model(file);
            let index = model
                .chunk()
                .unwrap()
                .descendants::<LuaIndexExpr>()
                .last()
                .unwrap();
            let typ = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
            assert_eq!(ws.humanize_type(typ), "PriorityGroup");
        }
    }
}

#[gtest]
fn table_index_computed_assignment_retains_unsafe_lookup_warning() {
    for statement in [
        "",
        "groups[key] = nil",
        "key = other_key",
        "groups = {}",
        "mutate()",
    ] {
        let mut ws = workspace(true);
        let source = format!(
            r#"
            ---@class PriorityGroup
            ---@field free_trains integer[]
            ---@param key string
            ---@param other_key string
            local function add(key, other_key)
                ---@type table<string, PriorityGroup>
                local groups = {{}}
                {initialize}
                {statement}
                local trains = groups[key].free_trains
            end
        "#,
            initialize = if statement.is_empty() {
                ""
            } else {
                "groups[key] = { free_trains = {} }"
            }
        );
        assert!(
            !ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            "{statement}"
        );
    }
}

#[gtest]
fn table_index_computed_guards_preserve_nested_read_contracts() {
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for table_index in [false, true] {
        for array_index in [false, true] {
            let mut ws = workspace(table_index);
            let mut config = ws.get_emmyrc();
            config.strict.array_index = array_index;
            ws.update_emmyrc(config);
            let source = r#"
                ---@class Dispatcher
                ---@field Provided table<string, table<integer, integer>>
                ---@param dispatcher Dispatcher
                ---@param key string
                ---@param id integer
                local function lookup(dispatcher, key, id)
                    if dispatcher.Provided[key] and dispatcher.Provided[key][id] then
                        local count = dispatcher.Provided[key][id]
                    end
                end
            "#;
            assert!(ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, source));
            let file = ws.def(source);
            let model = ws.analysis.semantic_model(file);
            let index = model
                .chunk()
                .unwrap()
                .descendants::<LuaIndexExpr>()
                .filter(|index| index.syntax().text() == "dispatcher.Provided[key][id]")
                .last()
                .unwrap();
            let typ = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
            assert_eq!(typ, ws.ty("integer"));

            let unguarded = r#"
                ---@param entries table<string, table<integer, integer>>
                ---@param key string
                ---@param id integer
                local function lookup(entries, key, id)
                    local count = entries[key][id]
                end
            "#;
            assert_eq!(
                ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, unguarded),
                !table_index
            );

            ws.def("---@type string[]\nguard_array = {}\n---@type table<string, string>\nguard_map = {}");
            let array = ws.expr_ty("guard_array[1]");
            let dictionary = ws.expr_ty("guard_map['key']");
            assert_eq!(array, ws.ty(if array_index { "string?" } else { "string" }));
            assert_eq!(
                dictionary,
                ws.ty(if table_index { "string?" } else { "string" })
            );
        }
    }
}

#[gtest]
fn table_index_computed_guards_discard_stale_facts() {
    for mutation in [
        "key = other_key",
        "dispatcher = other_dispatcher",
        "dispatcher.Provided = {}",
        "dispatcher.Provided[key] = nil",
        "local alias = dispatcher.Provided; alias[key] = nil",
        "mutate()",
        "local unused = mutate()",
        "local key = other_key",
    ] {
        let mut ws = workspace(true);
        let source = format!(
            r#"
            ---@class Dispatcher
            ---@field Provided table<string, table<integer, integer>>
            ---@param dispatcher Dispatcher
            ---@param other_dispatcher Dispatcher
            ---@param key string
            ---@param other_key string
            ---@param id integer
            local function lookup(dispatcher, other_dispatcher, key, other_key, id)
                if dispatcher.Provided[key] then
                    {mutation}
                    local count = dispatcher.Provided[key][id]
                end
            end
        "#
        );
        assert!(
            !ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            "{mutation}"
        );
    }
}

#[gtest]
fn table_index_computed_guards_preserve_asymmetric_fields() {
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for table_index in [false, true] {
        let source = r#"
            ---@class Dispatcher
            ---@field (read) Provided table<string, table<integer, integer>>
            ---@field (write) Provided table<string, table<integer, integer>>|string
            ---@param dispatcher Dispatcher
            ---@param key string
            ---@param id integer
            local function lookup(dispatcher, key, id)
                dispatcher.Provided = 'reset'
                if dispatcher.Provided[key] and dispatcher.Provided[key][id] then
                    local count = dispatcher.Provided[key][id]
                end
            end
        "#;
        let mut ws = workspace(table_index);
        assert!(ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source));
        assert!(ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, source));
        let file = ws.def(source);
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .filter(|index| index.syntax().text() == "dispatcher.Provided[key][id]")
            .last()
            .unwrap();
        let typ = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        assert_eq!(typ, ws.ty("integer"));
    }
}

// These combinations require the independent factory and directional patches.
#[gtest]
fn table_index_preserves_factory_and_directional_contracts() {
    for enabled in [false, true] {
        let mut ws = workspace(enabled);
        ws.def(
            r#"
            function make_lookup()
                return setmetatable({}, {
                    ---@param key string
                    ---@return string
                    __index = function(_, key) return key end
                })
            end
            fallback = make_lookup()
            ---@class Directional
            ---@field (read) required integer?
            ---@field (write) required integer
            ---@type Directional
            target = {}
            ---@type integer?
            value = nil
        "#,
        );
        expect_that!(ws.expr_ty("fallback.missing"), eq(&LuaType::String));
        expect_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::AssignTypeMismatch,
                "target.required = value"
            ),
            eq(false)
        );
    }
}
