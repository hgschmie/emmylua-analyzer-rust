use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

fn check(definition: &str, source: &str, code: DiagnosticCode, separate: bool) -> bool {
    let mut ws = VirtualWorkspace::new();
    if separate {
        ws.def(definition);
        ws.has_no_diagnostic(code, source)
    } else {
        ws.has_no_diagnostic(code, &format!("{definition}\n{source}"))
    }
}

const DEFINITION: &str = "---@enum demo.State\nstates = { reset = 0, running = 1 }";

#[test]
fn numeric_enum_members_are_valid_returns_and_assignments() {
    for prefix in ["", "local "] {
        let definition =
            format!("---@enum demo.State\n{prefix}states = {{ reset = 0, running = 1 }}");
        for value in ["states.reset", "states.running", "0", "1"] {
            assert!(
                check(
                    &definition,
                    &format!("---@return demo.State?\nlocal function f() return {value} end"),
                    DiagnosticCode::ReturnTypeMismatch,
                    false
                ),
                "return {value}"
            );
            assert!(
                check(
                    &definition,
                    &format!("---@type demo.State\nlocal value = {value}"),
                    DiagnosticCode::AssignTypeMismatch,
                    false
                ),
                "assignment {value}"
            );
        }
    }
}

#[test]
fn numeric_enum_aliases_and_foreign_definitions_preserve_membership() {
    let definition = format!(
        "{DEFINITION}\n---@alias StateAlias demo.State\n---@alias OptionalState StateAlias?"
    );
    for separate in [false, true] {
        for (value, valid) in [("0", true), ("1", true), ("99", false), ("nil", true)] {
            assert_eq!(
                check(
                    &definition,
                    &format!("---@return OptionalState\nlocal function f() return {value} end"),
                    DiagnosticCode::ReturnTypeMismatch,
                    separate
                ),
                valid,
                "{value} / separate={separate}"
            );
        }
        for (value, valid) in [("0", true), ("99", false)] {
            assert_eq!(
                check(
                    &definition,
                    &format!("---@type StateAlias\nlocal value = {value}"),
                    DiagnosticCode::AssignTypeMismatch,
                    separate
                ),
                valid
            );
        }
    }
}

#[test]
fn numeric_enum_returns_check_all_union_members() {
    for target in ["demo.State", "demo.State?", "StateAlias", "OptionalState"] {
        let definition = format!(
            "{DEFINITION}\n---@alias StateAlias demo.State\n---@alias OptionalState demo.State?"
        );
        let optional = target.ends_with('?') || target == "OptionalState";
        for (source, valid) in [
            ("0|1", true),
            ("0|99", false),
            ("99|nil", false),
            ("0|1|nil", optional),
            ("demo.State|nil", optional),
        ] {
            assert_eq!(
                check(
                    &definition,
                    &format!(
                        "---@param value {source}\n---@return {target}\nlocal function f(value) return value end"
                    ),
                    DiagnosticCode::ReturnTypeMismatch,
                    false
                ),
                valid,
                "{source} -> {target}"
            );
        }
    }
    assert!(check(
        DEFINITION,
        "---@param flag boolean\n---@return demo.State?\nlocal function f(flag) return flag and states.running or nil end",
        DiagnosticCode::ReturnTypeMismatch,
        false
    ));
    assert!(!check(
        DEFINITION,
        "---@param flag boolean\n---@return demo.State?\nlocal function f(flag) return flag and 99 or nil end",
        DiagnosticCode::ReturnTypeMismatch,
        false
    ));
}

#[test]
fn numeric_enum_parameters_reject_invalid_literals_and_unions() {
    for (source, valid) in [
        ("0", true),
        ("1", true),
        ("99", false),
        ("0|1", true),
        ("0|99", false),
        ("0|nil", false),
        ("demo.State|nil", false),
        ("99|nil", false),
    ] {
        let code = format!(
            "---@param state demo.State\nlocal function accept(state) end\n---@param value {source}\nlocal function f(value) accept(value) end"
        );
        assert_eq!(
            check(DEFINITION, &code, DiagnosticCode::ParamTypeMismatch, false),
            valid,
            "{source}"
        );
    }
}

#[test]
fn numeric_enum_membership_uses_the_correct_initializer() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(
        r#"
---@enum First
local first = { zero = 0, negative = -1, shift = 1 << 4 }
---@enum Second
second = { two = 2 }
---@enum Empty
local empty = {}
"#,
    );
    let first = ws.ty("First");
    let second = ws.ty("Second");
    let empty = ws.ty("Empty");
    let model = ws.analysis.semantic_model(file);
    for value in [0, -1, 16] {
        assert!(
            model.type_check(&LuaType::IntegerConst(value), &first),
            "{value}"
        );
        assert!(!model.type_check(&LuaType::IntegerConst(value), &second));
        assert!(!model.type_check(&LuaType::IntegerConst(value), &empty));
    }
    assert!(model.type_check(&LuaType::DocIntegerConst(2), &second));
    assert!(!model.type_check(&LuaType::DocIntegerConst(99), &second));
    assert!(!model.type_check(&LuaType::Integer, &first));
    assert!(!model.type_check(&LuaType::Number, &first));
}

#[test]
fn numeric_enum_table_field_owners_preserve_membership() {
    let definition = r#"
local container = {
    ---@enum Nested
    states = { zero = 0, one = 1 },
}
"#;
    for separate in [false, true] {
        for (value, valid) in [("0", true), ("1", true), ("99", false)] {
            assert_eq!(
                check(
                    definition,
                    &format!("---@return Nested\nlocal function f() return {value} end"),
                    DiagnosticCode::ReturnTypeMismatch,
                    separate
                ),
                valid,
                "{value} / separate={separate}"
            );
        }
    }
}
