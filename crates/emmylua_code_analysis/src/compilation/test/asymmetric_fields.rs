#![cfg(test)]
use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

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
