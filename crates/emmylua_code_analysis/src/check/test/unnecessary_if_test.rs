#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_vararg_unknown_and_boolean_conditions_are_not_constant() {
        let diagnostics = crate::check::test::check_source(
            r#"
            local function unknown(...)
                if ... then print('present') end
                assert(...)
            end
            ---@param ... boolean
            local function boolean(...)
                if ... then print('true') end
                assert(...)
            end
            unknown(false)
            boolean(false)
        "#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn test_vararg_false_conditions_still_report_impossible_branches() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param ... false
            local function always_false(...)
                if ... then print('unreachable') end
                assert(...)
            end
            always_false(false)
        "#,
        );
        let messages: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.code, d.message.as_str()))
            .collect();
        assert_eq!(
            messages,
            vec![
                (
                    DiagnosticCode::UnnecessaryAssert,
                    "Impossible assert: this expression is always falsy; prefer `error()`"
                ),
                (
                    DiagnosticCode::UnnecessaryIf,
                    "Impossible `if` statement: this condition is always falsy"
                ),
            ]
        );
    }

    #[test]
    fn test_issue_392() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf,
        r#"
        local a = false ---@type boolean|nil
        if a == nil or a then -- Unnecessary `if` statement: this condition is always truthy [unnecessary-if]
            print('a is not false')
        end
        "#
        ));
    }

    #[test]
    fn test_issue_396() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf,
        r#"
        local a = false ---@type 'a'|'b'
        if a ~= 'a' then -- Unnecessary `if` statement: this condition is always truthy [unnecessary-if]
        end
        "#
        ));
    }
}

#[test]
fn loop_exit_preserves_possible_boolean_assignments() {
    for body in [
        "for _, value in pairs(values) do if value then found = true end end",
        "for i = 1, limit do found = true end",
        "while flag do found = true; break end",
        "for _, value in pairs(values) do found = true; break end",
        "for _, value in pairs(values) do for _, other in pairs(values) do found = true end end",
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            "---@param values boolean[]\n---@param limit integer\n---@param flag boolean\nlocal function f(values, limit, flag)\nlocal found = false\n{body}\nif found then print(found) end\nend"
        );
        assert!(
            ws.has_no_diagnostic(crate::DiagnosticCode::UnnecessaryIf, &source),
            "{body}"
        );
    }
}

#[test]
fn loop_exit_retains_definite_and_unreachable_cases() {
    for body in [
        "for i = 2, 1 do found = true end",
        "while false do found = true end",
        "for _, value in pairs(values) do return end",
        "for _, value in pairs(values) do local found = true end",
        "for i = 1, 3 do found = true end",
        "repeat found = true until flag",
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            "---@param values boolean[]\n---@param flag boolean\nlocal function f(values, flag)\nlocal found = false\n{body}\nif found then print(found) end\nend"
        );
        assert!(
            !ws.has_no_diagnostic(crate::DiagnosticCode::UnnecessaryIf, &source),
            "{body}"
        );
    }
}
