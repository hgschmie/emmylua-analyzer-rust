//! Tests for the param_type_check / return_type_mismatch checkers.

use crate::DiagnosticCode;

use super::{check_source, count_by_code};

/// Argument type mismatch: f expects string, got 1.
#[test]
fn test_param_type_mismatch_basic() {
    let diagnostics = check_source("---@param s string\nlocal function f(s) end\nf(1)");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        1
    );
}

/// Argument types match: not reported.
#[test]
fn test_param_type_match_ok() {
    let diagnostics = check_source(
        "---@param s string\nlocal function f(s) end\nf('x')\nlocal n = 1\n---@param x number\nlocal function g(x) end\ng(n)",
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        0
    );
}

/// Named type parameter: passing number where Ref(C) is expected is reported (empty table {} matches any table type, so not reported).
#[test]
fn test_param_type_mismatch_named() {
    let diagnostics =
        check_source("---@class C\nlocal C = {}\n---@param c C\nlocal function f(c) end\nf(1)");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        1
    );
}

/// Undocumented function: skipped (nothing to check).
#[test]
fn test_param_type_no_doc_skipped() {
    let diagnostics = check_source("local function f(s) end\nf(1)");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        0
    );
}

/// Actual return type does not match the `---@return` annotation.
#[test]
fn test_return_type_mismatch_basic() {
    let diagnostics = check_source("---@return string\nlocal function f()\n    return 1\nend");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ReturnTypeMismatch),
        1
    );
}

/// Return types match: not reported.
#[test]
fn test_return_type_match_ok() {
    let diagnostics = check_source("---@return string\nlocal function f()\n    return 'x'\nend");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ReturnTypeMismatch),
        0
    );
}

/// No `@return` annotation: skipped.
#[test]
fn test_return_type_no_doc_skipped() {
    let diagnostics = check_source("local function f()\n    return 1\nend");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ReturnTypeMismatch),
        0
    );
}

/// `---@type number` annotation does not match the initializer.
#[test]
fn test_assign_type_mismatch_doc_type() {
    let diagnostics = check_source("---@type number\nlocal x = 's'");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::AssignTypeMismatch),
        1
    );
}

/// Annotation matches: not reported.
#[test]
fn test_assign_type_doc_type_ok() {
    let diagnostics = check_source("---@type number\nlocal x = 1");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::AssignTypeMismatch),
        0
    );
}

/// Reassignment does not match the declared type.
#[test]
fn test_assign_type_mismatch_reassign() {
    let diagnostics = check_source("local x = 1\nx = 's'");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::AssignTypeMismatch),
        1
    );
}

/// Reassignment matches: not reported.
#[test]
fn test_assign_type_reassign_ok() {
    let diagnostics = check_source("local x = 1\nx = 2");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::AssignTypeMismatch),
        0
    );
}

/// Undocumented local declaration initializer: nothing to compare, not reported.
#[test]
fn test_assign_type_no_doc_skipped() {
    let diagnostics = check_source("local x = 's'");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::AssignTypeMismatch),
        0
    );
}

// ──────────────────────────────────────────────
// generic_constraint_mismatch
// ──────────────────────────────────────────────

/// `---@generic T: string` + number argument → T violates the constraint.
#[test]
fn test_generic_constraint_mismatch_basic() {
    let diagnostics =
        check_source("---@generic T: string\n---@param x T\nlocal function f(x) end\nf(1)");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::GenericConstraintMismatch),
        1
    );
}

/// Argument satisfies the constraint: not reported.
#[test]
fn test_generic_constraint_match_ok() {
    let diagnostics =
        check_source("---@generic T: string\n---@param x T\nlocal function f(x) end\nf('s')");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::GenericConstraintMismatch),
        0
    );
}

/// Unconstrained generic: not reported.
#[test]
fn test_generic_no_constraint_skipped() {
    let diagnostics = check_source("---@generic T\n---@param x T\nlocal function f(x) end\nf(1)");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::GenericConstraintMismatch),
        0
    );
}

/// Named type constraint: `T: Base` with number passed.
#[test]
fn test_generic_constraint_named() {
    let diagnostics = check_source(
        "---@class Base\nlocal Base = {}\n---@generic T: Base\n---@param x T\nlocal function f(x) end\nf(1)",
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::GenericConstraintMismatch),
        1
    );
}

// ──────────────────────────────────────────────
// Alias expansion.
// ──────────────────────────────────────────────

/// `---@alias Dir -1|1` + argument 1: literal union collapses to number, not reported.
#[test]
fn test_alias_param_accepts_member() {
    let diagnostics =
        check_source("---@alias Dir -1|1\n---@param d Dir\nlocal function foo(d) end\nfoo(1)");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        0
    );
}

/// Alias target mismatch: Dir=number, string passed → reported.
#[test]
fn test_alias_param_rejects_mismatch() {
    let diagnostics =
        check_source("---@alias Dir number\n---@param d Dir\nlocal function foo(d) end\nfoo('s')");
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        1
    );
}

/// Recursive alias (A = B, B = A): guard prevents cycles, no crash.
#[test]
fn test_alias_recursive_no_crash() {
    let diagnostics = check_source(
        "---@alias A B\n---@alias B A\n---@param a A\nlocal function foo(a) end\nfoo(1)",
    );
    // Recursive alias cannot be decided: just ensure no crash (diagnostic count is not important).
    let _ = diagnostics;
}

#[test]
fn test_union_alias_parameter_members() {
    use crate::VirtualWorkspace;

    for parameter in ["Event|string", "EventType", "ChainedEvent"] {
        for actual in ["Event", "string", "Event|string"] {
            let source = format!(
                r#"
                ---@enum Event
                local Event = {{ one = 1 }}
                ---@alias EventType Event|string
                ---@alias ChainedEvent EventType
                ---@param event {parameter}
                local function raise_event(event) return event end
                ---@param event {actual}
                local function emit(event) raise_event(event) end
            "#
            );
            let mut ws = VirtualWorkspace::new();
            assert!(
                ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, &source),
                "{actual} -> {parameter}"
            );
        }
    }
}

#[test]
fn test_union_alias_parameter_rejects_incompatible_values() {
    for actual in ["boolean", "Event?", "Event|boolean", "Other"] {
        let diagnostics = check_source(&format!(
            r#"
            ---@enum Event
            local Event = {{ one = 1 }}
            ---@class Other
            local Other = {{}}
            ---@alias EventType Event|string
            ---@param event EventType
            local function raise_event(event) return event end
            ---@param event {actual}
            local function emit(event) raise_event(event) end
        "#
        ));
        assert_eq!(
            count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
            1,
            "{actual}: {diagnostics:#?}"
        );
    }
}

#[test]
fn test_union_alias_parameter_class_and_nullable() {
    let source = r#"
        ---@class Token
        local Token = {}
        ---@alias TokenType Token|string
        ---@alias OptionalToken TokenType|nil
        ---@param token TokenType
        local function accept(token) return token end
        ---@param token OptionalToken
        local function optional(token) return token end
        ---@param token Token
        ---@param nullable Token?
        local function use(token, nullable)
            accept(token)
            optional(nullable)
        end
    "#;
    let mut ws = crate::VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, source));
}

#[test]
fn test_union_alias_parameter_preserves_literal_diagnostics() {
    let diagnostics = check_source(
        r#"
        ---@class Token
        local Token = {}
        ---@alias Choice Token|"yes"
        ---@param value Choice
        local function accept(value) return value end
        accept("yes")
        accept("maybe")
    "#,
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        1,
        "{diagnostics:#?}"
    );
}

#[test]
fn test_union_alias_parameter_recursive_aliases() {
    let diagnostics = check_source(
        r#"
        ---@alias First Second|string
        ---@alias Second First|number
        ---@param value First
        local function accept(value) return value end
        accept("yes")
        accept(42)
        accept(false)
    "#,
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        1,
        "{diagnostics:#?}"
    );
}
