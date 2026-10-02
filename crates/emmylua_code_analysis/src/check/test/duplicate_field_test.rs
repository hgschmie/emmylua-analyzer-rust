#[cfg(test)]
mod test {
    use googletest::prelude::*;

    use crate::{DiagnosticCode, VirtualWorkspace};

    fn imported_functions_workspace() -> VirtualWorkspace {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "tools.lua",
            r#"
                ---@class DuplicateField.Tools
                local Tools = {}

                ---@param item {name: string}
                ---@return string
                function Tools.identifier(item)
                    return item.name
                end

                ---@return table<string, integer>
                function Tools.getInventory()
                    return {}
                end

                return Tools
            "#,
        );
        ws
    }

    #[gtest]
    fn imported_functions_read_inside_assignment_targets_are_not_redefined() {
        for assignment in [
            "inventory[tools.identifier(item)] = item",
            "inventory[tools[\"identifier\"](item)] = item",
            "inventory[tools.identifier] = item",
            "tools.getInventory().value = 1",
            "tools.getInventory()[tools.identifier(item)] = 1",
            "other, inventory[tools.identifier(item)] = 0, item",
            "inventory[tools.identifier(item)] = function() end",
            "inventory[tools.identifier(item)] = tools.identifier",
        ] {
            let mut ws = imported_functions_workspace();
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::DuplicateSetField,
                    &format!(
                        r#"
                        local tools = require("tools")
                        local inventory = {{}}
                        local item = {{name = "iron-plate"}}
                        local other
                        {assignment}
                        "#
                    ),
                ),
                eq(true),
                "{assignment}"
            );
        }
    }

    #[gtest]
    fn imported_function_reassignments_are_still_reported() {
        for assignment in [
            "tools.identifier = function(item) return item.name end",
            "tools[\"identifier\"] = function(item) return item.name end",
            "other, tools.identifier = 0, function(item) return item.name end",
            "function tools.identifier(item) return item.name end",
            "function tools:identifier(item) return item.name end",
        ] {
            let mut ws = imported_functions_workspace();
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::DuplicateSetField,
                    &format!(
                        r#"
                        local tools = require("tools")
                        local other
                        {assignment}
                        "#
                    ),
                ),
                eq(false),
                "{assignment}"
            );
        }
    }

    #[test]
    fn test_duplicate_field() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::DuplicateDocField,
            r#"
            ---@class Test
            ---@field name string
            ---@field name string
            local Test = {}

            Test.name = 1
            "#
        ));

        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::DuplicateDocField,
            r#"
            ---@class Test
            ---@field name string
            ---@field age number
            local Test = {}
            "#
        ));

        assert!(!ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::DuplicateDocField,
            r#"
            ---@class Test
            ---@field name string
            ---@field name number
            local Test = {}
            "#
        ));

        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::DuplicateDocField,
            r#"
            ---@class Test1
            ---@field name string

            ---@class Test2
            ---@field name string
            "#
        ));
    }

    #[test]
    fn test_duplicate_function_1() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::DuplicateDocField,
            r#"
            ---@class Test
            ---@field a fun()
            local Test = {}

            function Test.a()
            end
            "#
        ));

        for (first, second, overloads) in [
            (
                "fun(value: string): string",
                "fun(value: integer): integer",
                true,
            ),
            ("Callback<string>", "Callback<integer>", true),
            (
                "Identity<Callback<string>>",
                "Identity<Callback<integer>>",
                true,
            ),
            ("Choose<string>", "Choose<'fixed'>", true),
            ("Identity<string>", "Identity<integer>", false),
            ("Callback<string>", "Identity<integer>", false),
        ] {
            assert_eq!(
                ws.has_no_diagnostic_in_namespace(
                    DiagnosticCode::DuplicateDocField,
                    &format!(
                        r#"
                    ---@alias Callback<T> fun(value: T): T
                    ---@alias Identity<T> T
                    ---@alias Choose<T> T extends string and Callback<T> or integer
                    ---@class Test
                    ---@field a {first}
                    ---@field a {second}
                    local Test = {{}}

                    function Test.a()
                    end
                    "#
                    )
                ),
                overloads,
                "field types: {first}, {second}"
            );
        }

        assert!(!ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::DuplicateSetField,
            r#"
            ---@class Test
            ---@field a fun()
            local Test = {}

            function Test.a()
            end

            function Test.a()
            end
            "#
        ));
    }

    #[test]
    fn test_duplicate_function_2() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "1.lua",
            r#"
                ---@class D31.A
                local A = {}

                ---@param ... any
                ---@return any, any, any, any
                function A:execute(...)
                end

                return A
            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::DuplicateSetField,
            r#"
            local A = require("1")

            A.execute = function(trg, ...)
            end
        "#
        ));
    }

    #[test]
    fn test_duplicate_function_3() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::DuplicateSetField,
            r#"
                ---@class D31.A
                local A = {}
                A.a = function() end

                function A:init()
                    self.a = function()
                    end
                end
        "#
        ));
    }

    #[test]
    fn test_duplicate_function_4() {
        let mut ws = VirtualWorkspace::new();
        // Assigning to `.member` from a parameter is not reported.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::DuplicateSetField,
            r#"
                ---@class D31.A
                local A = {}
                A.a = function() end

                ---@param a fun()
                function A:init(a)
                    self.a = a
                end
        "#
        ));
    }

    #[test]
    fn test_return_self() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::DuplicateSetField,
            r#"
                ---@class test
                local A

                ---@return self
                function A.new()
                end

                function A:stop()
                end

                local a = A.new()

                a.stop = function()

                end
        "#
        ));
    }
}
