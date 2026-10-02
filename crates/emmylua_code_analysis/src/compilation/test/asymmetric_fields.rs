#![cfg(test)]
use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

#[googletest::gtest]
fn asymmetric_fields_inside_dictionaries_with_optional_keys() {
    use googletest::prelude::*;

    for table_index in [false, true] {
        for annotation in ["table<integer, Burner>", "{[integer]: Burner}"] {
            let mut ws = VirtualWorkspace::new();
            let mut config = ws.get_emmyrc();
            config.strict.table_index = table_index;
            ws.update_emmyrc(config);
            ws.def(DEFS);
            ws.def("---@type integer?\nkey = nil");
            ws.def(&format!(
                r#"
                ---@class OptionalDictionaryHolder
                ---@field (read) entries {annotation}
                ---@field (write) entries {annotation}|string
                ---@type OptionalDictionaryHolder
                holder = {{}}
                holder.entries = 'reset'
                selected = holder.entries[key]
                if selected then
                    selected.force = 'coal'
                    selected_force = selected.force
                    selected.fuel = nil
                    selected_fuel = selected.fuel
                end
            "#
            ));
            let burner = ws.ty("Burner?");
            expect_that!(ws.expr_ty("selected"), eq(&burner));
            let fuel = ws.ty("Fuel");
            expect_that!(ws.expr_ty("selected_force"), eq(&fuel));
            let optional_fuel = ws.ty("Fuel?");
            expect_that!(ws.expr_ty("selected_fuel"), eq(&optional_fuel));
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::UnnecessaryIf,
                    "local entry = holder.entries[key]\nif entry then end"
                ),
                eq(true)
            );
            for source in [
                "if key then holder.entries[key] = nil end",
                "local entry = holder.entries[key]\nif entry then entry.force = 'coal' end",
                "local entry = holder.entries[key]\nif entry then entry.fuel = nil end",
            ] {
                expect_that!(
                    ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
                    eq(true),
                    "{source}; tableIndex={table_index}"
                );
            }
            for source in [
                "if key then holder.entries[key] = 'wrong' end",
                "local entry = holder.entries[key]\nif entry then entry.force = nil end",
                "local entry = holder.entries[key]\nif entry then entry.force = false end",
            ] {
                expect_that!(
                    ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
                    eq(false),
                    "{source}; tableIndex={table_index}"
                );
            }
        }
    }
}

const DEFS: &str = r#"
---@meta
---@class Fuel
---@field name string
---@class Burner
---@field (read) fuel? Fuel
---@field (write) fuel? string|Fuel
---@field (read) force Fuel
---@field (write) force string|Fuel
---@field heat number
---@class Child: Burner
---@class Box<T, W>
---@field (read) value T
---@field (write) value W
---@type Burner
burner = {}
---@type Child
child = {}
---@type Box<Fuel, string>
box = {}
"#;

#[test]
fn asymmetric_fields_read_and_write() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFS);
    assert_eq!(ws.expr_ty("burner.force"), ws.ty("Fuel"));
    assert_eq!(ws.expr_ty("child.force"), ws.ty("Fuel"));
    assert_eq!(ws.expr_ty("box.value"), ws.ty("Fuel"));
    for source in [
        "burner.fuel = 'coal'",
        "burner['fuel'] = 'coal'",
        "burner.fuel = nil",
        "child.force = 'player'",
        "box.value = 'coal'",
        "burner.fuel, burner.heat = 'coal', 10",
    ] {
        assert!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
            "{source}"
        );
    }
    for source in [
        "burner.fuel = true",
        "burner.force = nil",
        "burner.force = true",
        "child.force = false",
        "box.value = 42",
        "burner.heat = 'bad'",
    ] {
        assert!(
            !ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
            "{source}"
        );
    }
}

#[test]
fn asymmetric_fields_flow_and_nested_writes() {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFS);
    ws.def(
        r#"
        burner.fuel = 'coal'
        after_write = burner.fuel
        if burner.fuel then
            guarded = burner.fuel.name
            burner.fuel = nil
            after_nil = burner.fuel
        end
        box.value = 'coal'
        generic_after = box.value
    "#,
    );
    assert_eq!(ws.expr_ty("after_write"), ws.ty("Fuel?"));
    assert_eq!(ws.expr_ty("guarded"), LuaType::String);
    assert_eq!(ws.expr_ty("after_nil"), ws.ty("Fuel?"));
    assert_eq!(ws.expr_ty("generic_after"), ws.ty("Fuel"));
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        "burner.force.name = 'ok'"
    ));
    assert!(!ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, "burner.force.name = 3"));
}

#[test]
fn asymmetric_fields_duplicate_rules() {
    let mut ws = VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(DiagnosticCode::DuplicateDocField, DEFS));
    for fields in [
        "---@field (read) value string\n---@field (read) value number",
        "---@field (write) value string\n---@field (write) value number",
        "---@field value string\n---@field (write) value number",
    ] {
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::DuplicateDocField,
            &format!("---@class Duplicate\n{fields}\nlocal t = {{}}")
        ));
    }
}

#[test]
fn asymmetric_fields_reindex_and_incomplete_directions() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file("defs.lua", DEFS);
    assert_eq!(ws.expr_ty("burner.force"), ws.ty("Fuel"));
    ws.def_file(
        "defs.lua",
        &DEFS.replace("(read) force Fuel", "(read) force number"),
    );
    assert_eq!(ws.expr_ty("burner.force"), LuaType::Number);
    ws.def("---@class Only\n---@field (write) value string\n---@type Only\nonly = {}");
    assert_eq!(ws.expr_ty("only.value"), LuaType::String);
    assert!(ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, "only.value = 'yes'"));
}

#[test]
fn asymmetric_fields_contextual_table_assignment() {
    let mut ws = VirtualWorkspace::new();
    ws.def(
        r#"
---@class Input
---@field name string
---@class Output
---@field result number
---@class Container
---@field (read) value Output
---@field (write) value Input
---@type Container
container = {}
"#,
    );
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        "container.value = { name = 'coal' }"
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        "container.value = { name = 3 }"
    ));
}

#[test]
fn asymmetric_fields_keys_and_independent_optionality() {
    let mut ws = VirtualWorkspace::new();
    ws.def(
        r#"
---@meta
---@class Keyed
---@field (read) ['odd-key'] number
---@field (write) ['odd-key']? string
---@field (read) [1] number
---@field (write) [1] string
---@type Keyed
keyed = {}
"#,
    );
    assert_eq!(ws.expr_ty("keyed['odd-key']"), LuaType::Number);
    assert_eq!(ws.expr_ty("keyed[1]"), LuaType::Number);
    assert!(ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, "keyed['odd-key'] = nil"));
    assert!(!ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, "keyed[1] = nil"));
    assert!(ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, "keyed[1] = 'ok'"));
    for fields in [
        "---@field (read,write) value number",
        "---@field (read) [string] number",
    ] {
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AnnotationUsageError,
            &format!("---@class Bad\n{fields}")
        ));
    }
}

#[test]
fn asymmetric_fields_callback_context() {
    let mut ws = VirtualWorkspace::new();
    ws.def(
        r#"
---@class CallbackHolder
---@field (read) callback fun(value: number)
---@field (write) callback fun(value: string)
---@type CallbackHolder
holder = {}
holder.callback = function(value)
    callback_value = value
end
"#,
    );
    assert_eq!(ws.expr_ty("callback_value"), LuaType::String);
}

// Compatibility coverage for strict.tableIndex lives on the work branch so the
// nullable dictionary lookup patch remains independently reviewable upstream.
#[googletest::gtest]
fn asymmetric_fields_dictionary_properties() {
    use googletest::prelude::*;

    for table_index in [false, true] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let mut config = ws.get_emmyrc();
        config.strict.table_index = table_index;
        ws.update_emmyrc(config);
        ws.def(
            r#"
            ---@class Result
            ---@field value number
            ---@class Input
            ---@field name string
            ---@field callback fun(value: string)
            ---@class DictionaryHolder
            ---@field (read) entries table<string, Result>
            ---@field (write) entries table<string, Input>|string
            ---@type DictionaryHolder
            holder = {}
            holder.entries = 'reset'
            after_setter = holder.entries
            lookup_after_setter = holder.entries.example
            holder.entries = {
                example = {
                    name = 'ok',
                    callback = function(value) dictionary_callback_value = value end,
                },
            }
        "#,
        );
        let dictionary = ws.ty("table<string, Result>");
        expect_that!(ws.expr_ty("after_setter"), eq(&dictionary));
        let result = ws.ty(if table_index { "Result?" } else { "Result" });
        expect_that!(ws.expr_ty("lookup_after_setter"), eq(&result));
        expect_that!(
            ws.expr_ty("dictionary_callback_value"),
            eq(&LuaType::String)
        );
        for source in [
            "holder.entries = 'reset'",
            "holder.entries = { example = { name = 'ok', callback = function(value) end } }",
            "holder.entries.example = { value = 42 }",
            "holder.entries.example = nil",
            "if holder.entries.example then holder.entries.example.value = 42 end",
        ] {
            expect_that!(
                ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
                eq(true),
                "{source}; tableIndex={table_index}"
            );
        }
        for source in [
            "holder.entries = nil",
            "holder.entries = { example = { name = 42, callback = function(value) end } }",
            "holder.entries.example = 'wrong'",
            "if holder.entries.example then holder.entries.example.value = 'wrong' end",
        ] {
            expect_that!(
                ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
                eq(false),
                "{source}; tableIndex={table_index}"
            );
        }
    }
}

#[googletest::gtest]
fn asymmetric_fields_inside_dictionaries() {
    use googletest::prelude::*;

    for table_index in [false, true] {
        for annotation in ["table<string, Burner>", "{[string]: Burner}"] {
            let mut ws = VirtualWorkspace::new();
            let mut config = ws.get_emmyrc();
            config.strict.table_index = table_index;
            ws.update_emmyrc(config);
            ws.def(DEFS);
            ws.def(&format!(
                r#"
                ---@type {annotation}
                burners = {{}}
                if burners.example then
                    burners.example.force = 'coal'
                    dictionary_force = burners.example.force
                    burners.example.fuel = nil
                    dictionary_fuel = burners.example.fuel
                end
            "#
            ));
            let fuel = ws.ty("Fuel");
            expect_that!(ws.expr_ty("dictionary_force"), eq(&fuel));
            let optional_fuel = ws.ty("Fuel?");
            expect_that!(ws.expr_ty("dictionary_fuel"), eq(&optional_fuel));
            for source in [
                "burners.example = nil",
                "if burners.example then burners.example.force = 'coal' end",
                "if burners.example then burners.example.fuel = nil end",
            ] {
                expect_that!(
                    ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
                    eq(true),
                    "{source}; tableIndex={table_index}"
                );
            }
            for source in [
                "if burners.example then burners.example.force = nil end",
                "if burners.example then burners.example.force = false end",
            ] {
                expect_that!(
                    ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, source),
                    eq(false),
                    "{source}; tableIndex={table_index}"
                );
            }
        }
    }
}
