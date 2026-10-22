#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_mutable_capture_across_callback_invocations() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            local tick = 0
            ---@param now integer
            function on_event(now)
                if tick == 0 then tick = now end
                tick = now
            end
        "#
        ));
    }

    #[test]
    fn test_mutable_capture_in_nested_and_sibling_callbacks() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            ---@param callback fun()
            local function debug_print(callback) callback() end
            local tick = 0
            ---@param now integer
            function on_event(now)
                debug_print(function()
                    if tick == 0 then tick = now end
                end)
                debug_print(function() tick = now end)
            end
            local ready = false
            function read_ready()
                if ready then print('ready') end
            end
            function write_ready() ready = true end
        "#
        ));
    }

    #[test]
    fn test_mutable_capture_preserves_local_flow_and_constants() {
        for source in [
            "local x = 0; function f() if x == 0 then print(x) end end",
            "function f() local x = 0; if x == 0 then print(x) end; x = 1 end",
            "local x = 0; function f() x = 2; if x == 2 then print(x) end end",
            "local x = 0; function f() if x == 0 then if x == 0 then print(x) end end; x = 1 end",
            "local x = 0; function f() local x = 1; x = 2 end; function g() if x == 0 then print(x) end end",
        ] {
            let mut ws = VirtualWorkspace::new();
            assert!(
                !ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, source),
                "{source}"
            );
        }
    }

    #[test]
    fn test_mutable_capture_reassigned_parameter_and_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            ---@param enabled boolean
            function factory(enabled)
                enabled = false
                return function()
                    if enabled then print('enabled') end
                    enabled = true
                end
            end
            local value
            function get_value()
                if value then print(value) end
            end
            function set_value() value = 'ready' end
        "#
        ));
    }

    #[test]
    fn test_mutable_capture_recursive_updates_and_definition_guards() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            local count = 0
            local enabled = false
            function update()
                if count == 0 then print('first') end
                if enabled then print('enabled') end
                count = count + 1
                enabled = not enabled
            end
            local value = nil
            function create_reader()
                if not value then
                    return function()
                        if value then print(value) end
                    end
                end
            end
            function write_value() value = 'changed' end
        "#
        ));
    }

    #[test]
    fn test_mutable_capture_retains_possible_nil_and_local_guard() {
        let source = r#"
            local value = { name = 'ready' }
            function clear() value = nil end
            function read()
                return value.name
            end
        "#;
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, source));
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            &source.replace("return value.name", "if value then return value.name end")
        ));
    }

    #[test]
    fn test_mutable_capture_multi_return_and_missing_assignment_slots() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            local first, second = 0, false
            ---@return integer, boolean
            local function values() return 1, true end
            function write_values() first, second = values() end
            function read_values()
                if first == 0 then print('zero') end
                if second then print('set') end
            end
            local a, b = 0, 'ready'
            function clear_second() a, b = 1 end
            function check_second() if b then print(b) end end
        "#
        ));
    }

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
