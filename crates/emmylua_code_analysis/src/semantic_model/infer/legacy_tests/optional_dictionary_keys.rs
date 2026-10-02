#![cfg(test)]

use googletest::prelude::*;

use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

const PROCESSOR_DEFS: &str = r#"
---@alias uint64 integer
---@class ProcInfo
---@field value number
---@alias ProcInfoTable table<uint64, ProcInfo>
---@class LuaEntity
---@field unit_number? uint64
---@type ProcInfoTable
procinfos = {}
---@type LuaEntity
processor = {}
"#;

#[gtest]
fn optional_property_key_preserves_dictionary_value() {
    let mut ws = VirtualWorkspace::new();
    ws.def(PROCESSOR_DEFS);
    let expected = ws.ty("ProcInfo?");
    expect_that!(
        ws.expr_ty("procinfos[processor.unit_number]"),
        eq(&expected)
    );
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            "local procinfo = procinfos[processor.unit_number]\nif procinfo then end"
        ),
        eq(true)
    );
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            "local procinfo = procinfos[processor.unit_number]\nif procinfo then local v = procinfo.value end"
        ),
        eq(true)
    );
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            "local v = procinfos[processor.unit_number].value"
        ),
        eq(false)
    );
}

#[gtest]
fn optional_keys_work_for_generic_and_object_dictionaries() {
    for (key_type, value_type) in [
        ("integer", "string"),
        ("string", "number"),
        ("boolean", "string"),
    ] {
        for dictionary in [
            format!("table<{key_type}, {value_type}>"),
            format!("{{[{key_type}]: {value_type}}}"),
        ] {
            let mut ws = VirtualWorkspace::new();
            ws.def(&format!(
                "---@type {dictionary}\nmap = {{}}\n---@type {key_type}?\nkey = nil"
            ));
            let expected = ws.ty(&format!("{value_type}?"));
            expect_that!(ws.expr_ty("map[key]"), eq(&expected));
            expect_that!(ws.expr_ty("map[nil]"), eq(&LuaType::Nil));
        }
    }
}

#[gtest]
fn aliases_and_multiline_unions_preserve_optional_keys() {
    let mut ws = VirtualWorkspace::new();
    ws.def(
        r#"
---@alias Id integer
---@alias OptionalId Id?
---@alias NestedId OptionalId
---@alias MultiId
---| Id
---| nil
---@type table<Id, string>
map = {}
---@type NestedId
nested = nil
---@type MultiId
multiline = nil
"#,
    );
    let expected = ws.ty("string?");
    expect_that!(ws.expr_ty("map[nested]"), eq(&expected));
    expect_that!(ws.expr_ty("map[multiline]"), eq(&expected));
}

#[gtest]
fn union_keys_combine_matching_signatures_and_missing_branches() {
    let mut ws = VirtualWorkspace::new();
    ws.def(
        r#"
---@type table<integer, string>
map = {}
---@type integer|boolean
partial_key = 0
---@type boolean|function
missing_key = false
---@type 1|2
either_number = 1
---@type {[string]: string, [integer]: number}
object = {}
---@type string|integer
both_keys = 0
---@type string|integer|boolean
mixed_keys = 0
---@type table<string|integer, boolean>
union_dictionary = {}
"#,
    );
    let partial = ws.ty("string?");
    let both = ws.ty("string|number");
    let mixed = ws.ty("string|number|nil");
    expect_that!(ws.expr_ty("map[partial_key]"), eq(&partial));
    expect_that!(ws.expr_ty("map[missing_key]"), eq(&LuaType::Nil));
    expect_that!(ws.expr_ty("map[either_number]"), eq(&LuaType::String));
    expect_that!(ws.expr_ty("object[both_keys]"), eq(&both));
    expect_that!(ws.expr_ty("object[mixed_keys]"), eq(&mixed));
    expect_that!(
        ws.expr_ty("union_dictionary[both_keys]"),
        eq(&LuaType::Boolean)
    );
}

#[gtest]
fn optional_literal_keys_preserve_required_object_fields() {
    let mut ws = VirtualWorkspace::new();
    ws.def(
        r#"
---@type {name: string, count: number}
object = {}
---@type 'name'?
optional_name = nil
---@type 'name'|'count'
either_field = 'name'
---@type 'name'|'missing'
sometimes_missing = 'name'
"#,
    );
    let optional = ws.ty("string?");
    let either = ws.ty("string|number");
    expect_that!(ws.expr_ty("object.name"), eq(&LuaType::String));
    expect_that!(ws.expr_ty("object[optional_name]"), eq(&optional));
    expect_that!(ws.expr_ty("object[either_field]"), eq(&either));
    expect_that!(ws.expr_ty("object[sometimes_missing]"), eq(&optional));
}

#[gtest]
fn guarded_and_asserted_keys_keep_nonnullable_lookup_behavior() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(PROCESSOR_DEFS);
    let expected = ws.ty("ProcInfo");
    expect_that!(
        ws.expr_ty("procinfos[assert(processor.unit_number)]"),
        eq(&expected)
    );
    ws.def(
        r#"
if processor.unit_number then
    guarded = procinfos[processor.unit_number]
end
local procinfo = procinfos[processor.unit_number]
if procinfo then
    guarded_value = procinfo.value
end
"#,
    );
    expect_that!(ws.expr_ty("guarded"), eq(&expected));
    expect_that!(ws.expr_ty("guarded_value"), eq(&LuaType::Number));
}

#[gtest]
fn optional_keys_do_not_weaken_dictionary_assignment_checks() {
    let mut ws = VirtualWorkspace::new();
    ws.def("---@type table<integer, string>\nmap = {}\n---@type integer?\nkey = nil");
    for source in [
        "if key then map[key] = 'ok' end",
        "if key then map[key] = nil end",
    ] {
        expect_that!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
            eq(true)
        );
    }
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            "if key then map[key] = 123 end"
        ),
        eq(false)
    );
}

#[gtest]
fn explicit_index_function_keeps_its_declared_return_type() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
function make_lookup()
    return setmetatable({}, {
        ---@param key string
        ---@return string
        __index = function(_, key) return key end,
    })
end
metatable_backed = make_lookup()
---@class Fallback
---@field [string] string
---@type Fallback
fallback = {}
"#,
    );
    expect_that!(ws.expr_ty("metatable_backed.missing"), eq(&LuaType::String));
    expect_that!(ws.expr_ty("fallback.missing"), eq(&LuaType::String));
}

#[gtest]
fn changing_key_optionality_reindexes_the_lookup() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file("defs.lua", PROCESSOR_DEFS);
    let nullable = ws.ty("ProcInfo?");
    expect_that!(
        ws.expr_ty("procinfos[processor.unit_number]"),
        eq(&nullable)
    );
    ws.def_file(
        "defs.lua",
        &PROCESSOR_DEFS.replace("unit_number? uint64", "unit_number uint64"),
    );
    let required = ws.ty("ProcInfo");
    expect_that!(
        ws.expr_ty("procinfos[processor.unit_number]"),
        eq(&required)
    );
    ws.def_file("defs.lua", PROCESSOR_DEFS);
    expect_that!(
        ws.expr_ty("procinfos[processor.unit_number]"),
        eq(&nullable)
    );
}
