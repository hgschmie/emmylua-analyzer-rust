use super::check_source;
use crate::DiagnosticCode;

fn assignment_errors(source: &str) -> Vec<super::Diagnostic> {
    check_source(source)
        .into_iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::AssignTypeMismatch)
        .collect()
}

#[test]
fn member_self_assignment_uses_declared_integer() {
    let mut ws = crate::VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@class Stop
        ---@field priority integer
        ---@param stop Stop
        local function update(stop)
            stop.priority = stop.priority or 0
        end
    "#,
    ));
}

#[test]
fn member_self_assignment_preserves_optional_and_generic_fields() {
    let mut ws = crate::VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@class Stop<T>
        ---@field priority T?
        ---@class Container
        ---@field stop Stop<integer>
        ---@param stop Stop<integer>
        ---@param container Container
        local function update(stop, container)
            stop.priority = stop.priority or 0
            container.stop.priority = container.stop.priority or 0
        end
    "#,
    ));
}

#[test]
fn member_self_assignment_rejects_invalid_fallbacks() {
    for fallback in ["'wrong'", "{}"] {
        let source = format!(
            r#"
            ---@class Stop
            ---@field priority integer?
            ---@param stop Stop
            local function update(stop)
                stop.priority = stop.priority or {fallback}
            end
        "#
        );
        let diagnostics = assignment_errors(&source);
        assert_eq!(diagnostics.len(), 1, "{fallback}: {diagnostics:?}");
    }
}

#[test]
fn member_self_assignment_does_not_use_a_future_write() {
    let mut ws = crate::VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@class Stop
        ---@field priority integer
        ---@param stop Stop
        local function update(stop)
            ---@type integer
            local previous = stop.priority
            stop.priority = 0
            return previous
        end
    "#,
    ));
}

#[test]
fn member_self_assignment_preserves_prior_write_narrowing() {
    let mut ws = crate::VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@class Stop
        ---@field priority integer|string
        ---@param stop Stop
        local function update(stop)
            stop.priority = 42
            ---@type integer
            local previous = stop.priority
            stop.priority = stop.priority or 0
            return previous
        end
    "#,
    ));
}

#[test]
fn member_self_assignment_rhs_precedes_all_parallel_writes() {
    let mut ws = crate::VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@class Stop
        ---@field priority integer
        ---@param stop Stop
        local function update(stop)
            ---@type integer
            local previous
            stop.priority, previous = 0, stop.priority
            return previous
        end
    "#,
    ));
}

#[test]
fn member_self_assignment_works_on_iteration_variables() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@class Stop
        ---@field priority integer
        ---@param stops table<integer, Stop>
        local function update(stops)
            for _, stop in pairs(stops) do
                stop.priority = stop.priority or 0
            end
        end
    "#
    ));
}

#[test]
fn member_self_assignment_rejects_noninteger_assignments() {
    let diagnostics = assignment_errors(
        r#"
        ---@class Stop
        ---@field priority integer
        ---@param stop Stop
        local function update(stop)
            stop.priority = 0.5
        end
    "#,
    );
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
}

#[test]
fn member_self_assignment_retains_nullable_rhs_contract() {
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};

    let mut ws = crate::VirtualWorkspace::new();
    let fid = ws.def_file(
        "nullable.lua",
        r#"
        ---@class Stop
        ---@field priority integer?
        ---@param stop Stop
        local function update(stop)
            stop.priority = stop.priority or 0
        end
    "#,
    );
    let expected = ws.ty("integer?");
    let model = ws.analysis.semantic_model(fid);
    let rhs = model
        .chunk()
        .unwrap()
        .descendants::<LuaIndexExpr>()
        .nth(1)
        .unwrap();
    assert_eq!(
        model.type_of_expr_at(rhs.get_syntax_id(), rhs.get_position()),
        expected
    );
}
