//! Prove that a numeric-loop body cannot run, without evaluating arbitrary code.

use emmylua_parser::{
    LuaAstNode, LuaExpr, LuaForStat, LuaLiteralToken, LuaStat, NumberResult, UnaryOperator,
};

pub(super) fn is_in_unentered_body(model: &super::SemanticModel, statement: &LuaStat) -> bool {
    let integer_runtime = model
        .lua_version()
        .is_some_and(|version| version >= emmylua_parser::LuaVersionNumber::new(5, 3, 0));
    statement.ancestors::<LuaForStat>().any(|loop_stat| {
        loop_stat
            .get_block()
            .is_some_and(|body| body.get_range().contains_range(statement.get_range()))
            && has_empty_range(&loop_stat, integer_runtime)
    })
}

fn has_empty_range(loop_stat: &LuaForStat, integer_runtime: bool) -> bool {
    // A zero step does not establish a value proof: runtime-error control flow
    // is outside this transfer, and older numeric-loop runtimes differ here.
    let mut expressions = loop_stat.get_iter_expr();
    let Some(start) = expressions.next().and_then(|value| literal_number(&value)) else {
        return false;
    };
    let Some(stop) = expressions.next().and_then(|value| literal_number(&value)) else {
        return false;
    };
    let step = match expressions.next() {
        Some(expr) => literal_number(&expr),
        None => Some(NumberResult::Int(1)),
    };
    let Some(step) = step else {
        return false;
    };
    // Lua 5.1/5.2 and LuaJIT cannot use the integer spelling to prove
    // distinct runtime bounds above floating-point's exact integer range.
    if integer_runtime
        && let (NumberResult::Int(start), NumberResult::Int(stop), NumberResult::Int(step)) =
            (&start, &stop, &step)
    {
        return (*step > 0 && start > stop) || (*step < 0 && start < stop);
    }
    // Mixed numeric forms use floating-point comparison only when integer
    // operands convert exactly. Unknown precision and zero steps keep the body.
    match (exact_float(start), exact_float(stop), exact_float(step)) {
        (Some(start), Some(stop), Some(step)) => {
            (step > 0.0 && start > stop) || (step < 0.0 && start < stop)
        }
        _ => false,
    }
}

const MAX_CONTIGUOUS_EXACT_INTEGER: u64 = 1_u64 << 53;

fn exact_float(value: NumberResult) -> Option<f64> {
    match value {
        NumberResult::Int(value) if value.unsigned_abs() <= MAX_CONTIGUOUS_EXACT_INTEGER => {
            Some(value as f64)
        }
        NumberResult::Float(value) if value.is_finite() => Some(value),
        _ => None,
    }
}

fn literal_number(expr: &LuaExpr) -> Option<NumberResult> {
    let mut expr = expr.clone();
    let mut negate = false;
    loop {
        expr = match expr {
            LuaExpr::ParenExpr(paren) => paren.get_expr()?,
            LuaExpr::UnaryExpr(unary) if unary.get_op_token()?.get_op() == UnaryOperator::OpUnm => {
                negate = !negate;
                unary.get_expr()?
            }
            LuaExpr::LiteralExpr(literal) => {
                let LuaLiteralToken::Number(number) = literal.get_literal()? else {
                    return None;
                };
                let value = match number.get_number_value() {
                    NumberResult::Int(value) => value,
                    // Out-of-range unsigned literals cannot certify an empty body.
                    NumberResult::Uint(value) => i64::try_from(value).ok()?,
                    NumberResult::Float(value) => {
                        return Some(NumberResult::Float(if negate { -value } else { value }));
                    }
                    NumberResult::Number => return None,
                };
                return Some(NumberResult::Int(if negate {
                    value.checked_neg()?
                } else {
                    value
                }));
            }
            _ => return None,
        };
    }
}
