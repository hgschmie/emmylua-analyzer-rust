use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

const INTEGER_ENUM: &str =
    "---@enum Direction\nlocal Direction = { north = 0, east = 4, south = 8, west = 12 }";
const CONTRACT_ENUM: &str = "---@enum Direction\nlocal Direction = {\n---@alias Direction.north integer\n---@type Direction.north\nnorth = nil,\n---@alias Direction.east integer\n---@type Direction.east\neast = nil,\n}";

fn expression_type(definition: &str, parameter: &str, expression: &str) -> LuaType {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(definition);
    ws.def(&format!(
        "---@param value {parameter}\nfunction run(value) result = {expression} end"
    ));
    ws.expr_ty("result")
}

#[test]
fn integer_enum_arithmetic_operators() {
    for expression in [
        "value + 4",
        "4 + value",
        "value - 4",
        "value * 2",
        "value % 16",
        "value // 2",
        "(value + 4) % 16",
        "(value - 4) % 16",
    ] {
        assert_eq!(
            expression_type(INTEGER_ENUM, "Direction", expression),
            LuaType::Integer,
            "{expression}"
        );
    }
    for expression in ["value / 2", "value ^ 2", "value + 1.5"] {
        assert_eq!(
            expression_type(INTEGER_ENUM, "Direction", expression),
            LuaType::Number,
            "{expression}"
        );
    }
}

#[test]
fn integer_enum_arithmetic_declared_member_contracts() {
    for expression in [
        "value + 4",
        "value - 4",
        "value * 2",
        "value % 16",
        "(value + 4) % 16",
    ] {
        assert_eq!(
            expression_type(CONTRACT_ENUM, "Direction", expression),
            LuaType::Integer,
            "{expression}"
        );
    }
}

#[test]
fn integer_enum_arithmetic_aliases() {
    let definitions = format!("{INTEGER_ENUM}\n---@alias Named Direction\n---@alias Generic<T> T");
    for parameter in ["Named", "Generic<Direction>", "Direction|integer"] {
        assert_eq!(
            expression_type(&definitions, parameter, "value % 16"),
            LuaType::Integer,
            "{parameter}"
        );
    }
}

#[test]
fn integer_enum_arithmetic_rejects_noninteger_evidence() {
    for definition in [
        "---@enum Direction\nlocal Direction = { a = nil }",
        "---@enum Direction\nlocal Direction = { a = 1, b = 'east' }",
        "---@enum Direction\nlocal Direction = { a = 1.5 }",
        "---@enum Direction\nlocal Direction = { a = {} }",
        "---@enum Direction\nlocal Direction = { a = missing }",
        "---@enum Direction\nlocal Direction = {\n---@type integer?\na = nil }",
        "---@enum Direction\nlocal Direction = {\n---@type number\na = nil }",
        "---@enum Direction\nlocal Direction = {\n---@type Direction\na = nil }",
    ] {
        assert_ne!(
            expression_type(definition, "Direction", "value + 4"),
            LuaType::Integer,
            "{definition}"
        );
    }
    for parameter in [
        "Direction?",
        "Direction|string",
        "Direction|number",
        "unknown",
    ] {
        assert_ne!(
            expression_type(INTEGER_ENUM, parameter, "value % 16"),
            LuaType::Integer,
            "{parameter}"
        );
    }
}

#[test]
fn integer_enum_arithmetic_keeps_operator_overloads() {
    assert_eq!(
        expression_type(
            "---@class Direction\n---@operator add(integer): string",
            "Direction",
            "value + 4"
        ),
        LuaType::String
    );
    assert_ne!(
        expression_type("---@class Direction", "Direction", "value + 4"),
        LuaType::Integer
    );
}

#[test]
fn integer_enum_arithmetic_contract_argument_compatibility() {
    for (parameter, accepted) in [
        ("integer", true),
        ("Direction.north", true),
        ("number", false),
        ("string", false),
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(CONTRACT_ENUM);
        let source = format!(
            "---@param target Direction\nlocal function consume(target) end\n---@param value {parameter}\nlocal function run(value) consume(value) end"
        );
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, &source),
            accepted,
            "{parameter}"
        );
    }
}

#[test]
fn integer_enum_arithmetic_cross_file_contracts() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def_file("direction.lua", CONTRACT_ENUM);
    ws.def("---@param value Direction\nfunction run(value) result = (value + 4) % 16 end");
    assert_eq!(ws.expr_ty("result"), LuaType::Integer);
}

#[test]
fn integer_enum_arithmetic_declared_return_compatibility() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(CONTRACT_ENUM);
    assert!(ws.has_no_diagnostic(DiagnosticCode::ReturnTypeMismatch, "---@param value Direction\n---@return Direction\nlocal function rotate(value) return (value - 4) % 16 end"));
    assert!(!ws.has_no_diagnostic(DiagnosticCode::ReturnTypeMismatch, "---@param value number\n---@return Direction\nlocal function invalid(value) return value end"));
}
