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

#[test]
fn multiline_integer_alias_arithmetic() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def(
        r#"
        ---@alias MultilineCount
        ---| 1 # First allowed integer.
        ---| 2 # Second allowed integer.
        ---@alias MultilineNumber
        ---| integer # Integer alternative.
        ---| number # Floating alternative must remain broad.
        ---@alias MultilineString
        ---| integer # Integer alternative.
        ---| string # Nonnumeric alternative must not establish integer evidence.
        ---@param count MultilineCount
        ---@param mixed_number MultilineNumber
        ---@param mixed_string MultilineString
        local function calculate(count, mixed_number, mixed_string)
            local sum = count + 1
            local difference = count - 1
            local product = count * 2
            local remainder = count % 2
            local quotient = count // 2
            local floating = mixed_number + 1
            local invalid_numeric = mixed_string + 1
        end
    "#,
    );
    let model = ws.analysis.semantic_model(file);
    for (name, expected) in [
        ("sum", LuaType::Integer),
        ("difference", LuaType::Integer),
        ("product", LuaType::Integer),
        ("remainder", LuaType::Integer),
        ("quotient", LuaType::Integer),
        ("floating", LuaType::Number),
        ("invalid_numeric", LuaType::Number),
    ] {
        let decl = model
            .decls()
            .unwrap()
            .iter()
            .find(|decl| decl.name == name)
            .unwrap();
        assert_eq!(model.type_of_decl(&decl.id), Some(expected), "{name}");
    }
}

#[test]
fn multiline_integer_alias_loop_variables() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for (start, step, expected) in [
        ("count", "1", LuaType::Integer),
        ("1", "count", LuaType::Integer),
        ("mixed_number", "1", LuaType::Number),
        ("1", "mixed_number", LuaType::Number),
        ("mixed_string", "1", LuaType::Number),
        ("1", "mixed_string", LuaType::Number),
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(&format!(
            r#"
            ---@alias MultilineCount
            ---| 1 # First allowed integer.
            ---| 2 # Second allowed integer.
            ---@alias MultilineNumber
            ---| integer # Integer alternative.
            ---| number # Floating alternative must remain broad.
            ---@alias MultilineString
            ---| integer # Integer alternative.
            ---| string # Nonnumeric alternative must remain broad.
            ---@param count MultilineCount
            ---@param mixed_number MultilineNumber
            ---@param mixed_string MultilineString
            local function visit(count, mixed_number, mixed_string)
                for i = {start}, 10, {step} do print(i) end
            end
        "#
        ));
        let model = ws.analysis.semantic_model(file);
        let variable = model
            .chunk()
            .unwrap()
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

#[test]
fn multiline_integer_operand_representation() {
    use crate::LuaMultiLineUnion;
    use crate::semantic_model::infer::vm::{binary_type, is_integer_operand};
    use emmylua_parser::BinaryOperator;
    use std::rc::Rc;

    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def("");
    let model = ws.analysis.semantic_model(file);
    for (members, integer) in [
        (
            vec![LuaType::IntegerConst(1), LuaType::DocIntegerConst(2)],
            true,
        ),
        (vec![LuaType::Integer, LuaType::Number], false),
        (vec![LuaType::Integer, LuaType::String], false),
        (vec![LuaType::Integer, LuaType::Nil], false),
    ] {
        // Keep documented alternatives as their multiline representation rather than parser-normalized Union.
        let operand = LuaType::MultiLineUnion(Rc::new(LuaMultiLineUnion::new(
            members
                .into_iter()
                .map(|ty| (ty, Some("Documented alternative".into())))
                .collect(),
        )));
        for op in [
            BinaryOperator::OpAdd,
            BinaryOperator::OpSub,
            BinaryOperator::OpMul,
            BinaryOperator::OpMod,
            BinaryOperator::OpIDiv,
        ] {
            assert_eq!(
                binary_type(&model, op, &operand, &LuaType::Integer),
                if integer {
                    LuaType::Integer
                } else {
                    LuaType::Number
                },
                "{op:?}: {operand:?}"
            );
        }
        assert_eq!(
            is_integer_operand(&model, &operand, &mut Vec::new()),
            integer,
            "numeric-for operand contract: {operand:?}"
        );
    }
}
