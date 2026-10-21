use googletest::prelude::*;

use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

#[gtest]
fn integer_alias_arithmetic_preserves_integer_results() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@alias uint32 integer
        ---@alias Count uint32
        ---@alias Identity<T> T
        ---@alias SmallCount 1|2
        ---@type uint32
        x = 0
        ---@type Count
        y = 0
        ---@type Identity<uint32>
        generic_count = 0
        ---@type SmallCount
        small_count = 1
        "#,
    );
    for expression in [
        "x - y",
        "x + y",
        "x * y",
        "x - 1",
        "1 + x",
        "generic_count - x",
        "small_count + x",
    ] {
        expect_that!(
            ws.expr_ty(expression),
            eq(&LuaType::Integer),
            "{expression}"
        );
    }
}

#[gtest]
fn integer_alias_arithmetic_respects_optional_guards_and_assignments() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            r#"
            ---@alias uint32 integer
            ---@param amount uint32?
            ---@param count uint32
            local function subtract(amount, count)
                if amount then
                    amount = amount - count
                    ---@type uint32
                    local remaining = amount
                end
            end
            "#,
        ),
        eq(true)
    );
}

#[gtest]
fn integer_alias_arithmetic_does_not_turn_other_numbers_into_integers() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@alias uint32 integer
        ---@alias Float number
        ---@alias MaybeCount uint32?
        ---@class NominalCount: integer
        ---@type uint32
        x = 0
        ---@type Float
        floating = 0
        ---@type MaybeCount
        optional = nil
        ---@type NominalCount
        nominal = nil
        "#,
    );
    for expression in [
        "x - floating",
        "floating + x",
        "x * 0.5",
        "x / x",
        "x ^ x",
        "optional - x",
        "nominal - x",
    ] {
        expect_that!(ws.expr_ty(expression), eq(&LuaType::Number), "{expression}");
    }
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            "---@type uint32\nlocal invalid = x - floating"
        ),
        eq(false)
    );
}

#[gtest]
fn integer_alias_arithmetic_preserves_operator_overloads() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@class Vector
        ---@operator sub(Vector): string
        ---@alias VectorAlias Vector
        ---@type VectorAlias
        left = nil
        ---@type VectorAlias
        right = nil
        "#,
    );
    expect_that!(ws.expr_ty("left - right"), eq(&LuaType::String));
}

#[gtest]
fn integer_alias_arithmetic_stops_at_alias_cycles() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@alias First Second
        ---@alias Second First
        ---@type First
        cycle = nil
        "#,
    );
    expect_that!(ws.expr_ty("cycle - 1"), eq(&LuaType::Any));
}

#[test]
fn numeric_for_preserves_integer_aliases() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@alias uint32 integer
        ---@alias Count uint32
        ---@alias Identity<T> T
        ---@param value integer
        local function consume(value) end
        ---@param count Count
        ---@param step Identity<uint32>
        local function visit(count, step)
            for i = count, 1, -1 do consume(i) end
            for i = count, 100 do consume(i) end
            for i = 1, count, step do consume(i) end
        end
    "#
    ));
}

#[test]
fn numeric_for_alias_variable_types() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for (start, step, expected) in [
        ("count", "1", LuaType::Integer),
        ("1", "count", LuaType::Integer),
        ("fraction", "1", LuaType::Number),
        ("1", "fraction", LuaType::Number),
        ("mixed", "1", LuaType::Number),
        ("cycle", "1", LuaType::Number),
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(&format!(
            r#"
            ---@alias Count integer
            ---@alias Fraction number
            ---@alias Mixed integer|number
            ---@alias Cycle Cycle
            ---@param count Count
            ---@param fraction Fraction
            ---@param mixed Mixed
            ---@param cycle Cycle
            local function visit(count, fraction, mixed, cycle)
                for i = {start}, 10, {step} do print(i) end
            end
        "#
        ));
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let variable = chunk
            .descendants::<LuaNameExpr>()
            .find(|name| name.get_name_text().as_deref() == Some("i"))
            .unwrap();
        assert_eq!(
            model.type_of_expr(variable.get_syntax_id()),
            expected,
            "start={start}, step={step}"
        );
    }
}
