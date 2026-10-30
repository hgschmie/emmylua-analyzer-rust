#[cfg(test)]
mod test {
    use crate::check::test::check_source;
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_array_parent_accepts_empty_and_valid_elements() {
        for parent in ["Item[]", "ItemArray", "ItemArrayAlias", "ArrayBase<Item>"] {
            for value in ["{}", "{{name = 'ok'}}"] {
                let diagnostics = check_source(&format!(
                    r#"
---@class Item
---@field name string
---@class ItemArray: Item[]
---@alias ItemArrayAlias Item[]
---@class ArrayBase<T>: T[]
---@class Items: {parent}
---@type Items
local items = {value}
---@param values Items
local function accept(values) return values end
accept({value})
return items
"#
                ));
                assert!(diagnostics.is_empty(), "{parent}, {value}: {diagnostics:?}");
            }
        }
    }

    #[test]
    fn test_array_parent_checks_container_fields_separately() {
        for (value, expected) in [
            ("{name = 1, label = 'items', {name = 'ok'}}", vec![]),
            ("{name = 1, label = 'items'}", vec![]),
            ("{label = 'items'}", vec![DiagnosticCode::MissingFields]),
            ("{name = 1}", vec![DiagnosticCode::MissingFields]),
        ] {
            let diagnostics = check_source(&format!(
                r#"
---@class Item
---@field name string
---@class Metadata
---@field label string
---@class Items: Item[], Metadata
---@field name number
---@type Items
local items = {value}
return items
"#
            ));
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                expected,
                "{value}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn test_array_parent_reports_missing_field_on_element() {
        for (parent, value) in [
            ("Item[]", "{{}}"),
            ("Item[]", "{{name = 'ok'}, {}}"),
            ("Item[][]", "{{{}}}"),
        ] {
            let source = format!(
                r#"
---@class Item
---@field name string
---@class Items: {parent}
---@type Items
local items = {value}
return items
"#
            );
            let diagnostics = check_source(&source);
            assert_eq!(diagnostics.len(), 1, "{value}: {diagnostics:?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.code, DiagnosticCode::MissingFields);
            assert_eq!(diagnostic.message, "Missing required fields: `name`");
            assert_eq!(&source[diagnostic.range], "{}");
        }
    }

    #[test]
    fn test_array_parent_checks_elements_in_calls_and_reassignments() {
        for statement in ["accept({{}})", "items = {{}}"] {
            let source = format!(
                r#"
---@class Item
---@field name string
---@class Items: Item[]
---@type Items
local items
---@param values Items
local function accept(values) return values end
{statement}
return items, accept
"#
            );
            let diagnostics = check_source(&source);
            assert_eq!(diagnostics.len(), 1, "{statement}: {diagnostics:?}");
            assert_eq!(diagnostics[0].code, DiagnosticCode::MissingFields);
            assert_eq!(&source[diagnostics[0].range], "{}");
        }
    }

    #[test]
    fn test_array_parent_rejects_invalid_element_value() {
        let diagnostics = check_source(
            r#"
---@class Item
---@field name string
---@class Items: Item[]
---@type Items
local items = {{name = 123}}
return items
"#,
        );
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![DiagnosticCode::AssignTypeMismatch],
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_generic_index_signature_contracts() {
        for (value, expected) in [
            ("{integer = true, [1] = 'ok'}", vec![]),
            (
                "{integer = true, [1] = false}",
                vec![DiagnosticCode::AssignTypeMismatch],
            ),
            (
                "{integer = true, ['1'] = 'ok'}",
                vec![DiagnosticCode::InjectField],
            ),
            (
                "{[1] = 'ok'}",
                vec![
                    DiagnosticCode::AssignTypeMismatch,
                    DiagnosticCode::MissingFields,
                ],
            ),
        ] {
            let diagnostics = check_source(&format!(
                "
                ---@class Base<K, V>
                ---@field [K] V

                ---@class Record: Base<integer, string>
                ---@field integer boolean
                ---@type Record
                local value = {value}
                return value"
            ));
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                expected,
                "{value}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn test_generic_index_signature_on_call_argument() {
        let diagnostics = check_source(
            "---@class Dictionary<K, V>
            ---@field [K] V
            ---@param value Dictionary<string, boolean>
            local function accept(value) return value end
            accept({name = true})
            accept({name = 'bad'})",
        );
        assert!(
            diagnostics.iter().any(|d| matches!(
                d.code,
                DiagnosticCode::AssignTypeMismatch | DiagnosticCode::ParamTypeMismatch
            )),
            "{diagnostics:?}"
        );
        assert!(
            !diagnostics.iter().any(|d| matches!(
                d.code,
                DiagnosticCode::InjectField | DiagnosticCode::MissingFields
            )),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_index_signature_float_keys_use_numeric_domain() {
        for (domain, expected) in [
            ("integer", vec![DiagnosticCode::InjectField]),
            ("number", vec![]),
        ] {
            let diagnostics = check_source(&format!(
                "---@class Record
                ---@field [{domain}] string
                ---@type Record
                local value = {{[1.5] = 'ok'}}
                return value"
            ));
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                expected,
                "{domain}: {diagnostics:?}"
            );
        }
    }

    mod generic_tables {
        use super::*;

        use DiagnosticCode::ParamTypeMismatch;

        fn assert_diagnostics(source: &str, expected: &[DiagnosticCode]) {
            let diagnostics = check_source(source);
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                expected,
                "{source}\n{diagnostics:?}"
            );
        }

        #[test]
        fn test_record_constructors_ignore_overwritten_nested_values() {
            for constructor in [
                "{item = {}, [key] = {required = 'ok'}}",
                "({item = {}, [key] = ({required = 'ok'})})",
            ] {
                assert_diagnostics(
                    &format!(
                        "local key = 'item'\n---@param opts {{item: {{required: string}}}}\nlocal function accept(opts) return opts end\naccept({constructor})\npcall(accept, {constructor})\n---@type {{item: {{required: string}}}}\nlocal value = {constructor}\nreturn value"
                    ),
                    &[],
                );
            }
        }

        #[test]
        fn test_object_index_signatures_allow_nil_entries() {
            assert_diagnostics(
                r#"
                ---@param opts {values: table<string, string>, [string]: string}
                local function accept(opts) return opts end
                accept({values = {}, label = nil})
                "#,
                &[],
            );
        }

        #[test]
        fn test_generic_table_overload_requires_the_whole_call_to_match() {
            assert_diagnostics(
                r#"
                ---@param values table<integer, string>
                ---@param mode string
                ---@overload fun(values: table<string, integer>, mode: integer)
                local function accept(values, mode) return values, mode end
                accept({'a'}, 'a')
                accept({count = 1}, 1)
                accept({count = 1}, 'a')
                "#,
                &[ParamTypeMismatch],
            );
        }
    }

    #[test]
    fn test_class_method_self_field_diagnostic() {
        let diagnostics = check_source(
            r#"
            ---@class LinkedTableA
            local M

            function M:init()
                self.a = 1
            end
            "#,
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != DiagnosticCode::InjectField),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_class_method_self_field_reference_is_rejected() {
        let diagnostics = check_source(
            r#"
            ---@class LinkedTableA

            ---@type LinkedTableA
            local M

            function M:init()
                self.a = 1
            end
            "#,
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::InjectField),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_table_literal_integral_float_keys() {
        let diagnostics = check_source(
            r#"
            ---@class Numeric
            ---@field [0] integer
            ---@field [1] integer
            ---@type Numeric
            local decimal = {[0.0] = 0, [1.0] = 1}
            ---@type Numeric
            local exponent = {[0e0] = 0, [1e0] = 1}

            ---@class LargeNumeric
            ---@field [9223372036854774784] integer
            ---@type LargeNumeric
            local large = {[9223372036854774784.0] = 1}
            ---@param value Numeric
            local function accept(value) return value end
            accept({[0.0] = 0, [1e0] = 1})
            return decimal, exponent, large
            "#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn test_table_literal_integral_float_key_rejects_wrong_value() {
        for key in ["1", "9223372036854774784"] {
            let diagnostics = check_source(&format!(
                r#"
                ---@class Numeric
                ---@field [{key}]? integer
                ---@type Numeric
                local value = {{[{key}.0] = "bad"}}
                return value
                "#,
            ));
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                vec![DiagnosticCode::AssignTypeMismatch],
                "{key}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn test_table_literal_float_key_must_match_integer_key() {
        for (field, key) in [
            ("1", "1.5"),
            ("9223372036854775807", "9223372036854775808.0"),
        ] {
            let diagnostics = check_source(&format!(
                r#"
                ---@class Numeric
                ---@field [{field}]? integer
                ---@type Numeric
                local value = {{[{key}] = 1}}
                return value
                "#,
            ));
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                vec![DiagnosticCode::InjectField],
                "{key}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn test_table_literal_numeric_and_string_keys_are_distinct() {
        for (field, key) in [("1", "\"[1]\""), ("\"[1]\"", "1")] {
            let diagnostics = check_source(&format!(
                r#"
                ---@class Record
                ---@field [{field}] integer
                ---@type Record
                local value = {{[{key}] = 1}}
                return value
                "#,
            ));
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                vec![DiagnosticCode::InjectField, DiagnosticCode::MissingFields],
                "{field}, {key}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn test_table_literal_numeric_and_string_keys_inherited() {
        let diagnostics = check_source(
            r#"
            ---@class Numeric
            ---@field [1] integer
            ---@class Mixed: Numeric
            ---@field ["[1]"] integer
            ---@type Mixed
            local t = {["[1]"] = 1}
            return t
            "#,
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|d| (d.code, d.message.as_str()))
                .collect::<Vec<_>>(),
            vec![(
                DiagnosticCode::MissingFields,
                "Missing required fields: `[1]`"
            )],
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_table_literal_dictionary_index() {
        let diagnostics = check_source(
            r#"
            ---@class Dict
            ---@field [string] integer
            ---@type Dict
            local t = {foo = 1}
            ---@type Dict
            local empty = {}
            return t.foo, empty
            "#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn test_table_literal_dictionary_index_inherited() {
        let diagnostics = check_source(
            r#"
            ---@class Dict
            ---@field [string] integer
            ---@class NamedDict: Dict
            ---@field required integer
            ---@type NamedDict
            local t = {required = 1, foo = 2}
            return t
            "#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn test_table_literal_dictionary_index_rejects_wrong_key() {
        let diagnostics = check_source(
            r#"
            ---@class Dict
            ---@field [string] integer
            ---@type Dict
            local t = {[1] = 1}
            return t
            "#,
        );
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![DiagnosticCode::InjectField],
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_table_literal_dictionary_index_rejects_wrong_value() {
        let diagnostics = check_source(
            r#"
            ---@class Dict
            ---@field [string] integer
            ---@type Dict
            local t = {foo = "bad"}
            return t
            "#,
        );
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![DiagnosticCode::AssignTypeMismatch],
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_table_literal_dictionary_index_requires_named_field() {
        let diagnostics = check_source(
            r#"
            ---@class Dict
            ---@field [string] integer
            ---@field required integer
            ---@type Dict
            local t = {foo = 1}
            return t
            "#,
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|d| (d.code, d.message.as_str()))
                .collect::<Vec<_>>(),
            vec![(
                DiagnosticCode::MissingFields,
                "Missing required fields: `required`"
            )],
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_issue_195() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            local ret = {} --- @type string[]
            local b
            ret[#ret + 1] = b
            ret[#ret + 1] = 'a'
        "#
        ));
    }

    #[test]
    fn test_inject_field() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            ---@class test1

            ---@type test1
            local test
            test.a = 1

        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            ---@class test2
            ---@field a number

            ---@type test2
            local test
            test.a = 1

        "#
        ));
    }

    #[test]
    fn test_class_def_dynamic_key_in_method_is_allowed() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            ---@class TestBind
            Bind = {}

            ---@class TestFunc
            ---@field call_name string
            local M = {}

            function M:call()
                Bind[self.call_name] = 1
            end
        "#
        ));
    }

    #[test]
    fn test_super_table() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            ---@class test1<T>: {[string]: number }, table<string, string>

            ---@type test1<string>
            local test

            test.a = "1"
        "#
        ));
    }

    #[test]
    fn test_object() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            ---@type { [number]: number }
            local test2 = {
            }
            test2.a = 1
        "#
        ));
    }

    #[test]
    fn test_self() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            ---@class Diagnostic.8_1
            ---@field a number
            local Test = {}

            function Test:name()
                self.a = 1
            end
        "#
        ));
    }

    #[test]
    fn test_any_key() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            ---@type { [number]: number }
            local t

            t[any] = 1
        "#
        ));
    }

    #[test]
    fn test_issue_264() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
                local a = { 'a' }
                a[#a + 1] = 'b'

                ---@type string[]
                local b = { 'a' }
                b[#b + 1] = 'b'

                ---@type table<integer, string>
                local c = { 'a' }
                c[#c + 1] = 'b'

                ---@type { [integer]: string }
                local d = { 'a' }
                d[#d + 1] = 'b'
        "#
        ));
    }

    #[test]
    fn test_tuple() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
                local a = { 'a' }
                a[#a + 1] = 'b'
        "#
        ));

        // assert!(!ws.has_no_diagnostic(
        //     DiagnosticCode::InjectField,
        //     r#"
        //         ---@type [ 'a' ]
        //         local a = { 'a' }
        //         a[#a + 1] = 'b'
        // "#
        // ));
    }

    #[test]
    fn test_export() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
            local export = {}

            export.a = 1

            return export
            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            local a = require("a")
            a.newField = 1
            "#,
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            local a = require("a")
            a.a = 2
            "#,
        ));
    }

    #[test]
    fn test_export_2() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
            return {
                a = 1
            }
            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            local a = require("a")
            a.newField = 1
            "#,
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            local a = require("a")
            a.a = 2
            "#,
        ));
    }

    #[test]
    fn test_issue_660() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "a.lua",
            r#"
            --- @class (file) vim.var_accessor
            --- @field [string] any
            --- @field [integer] vim.var_accessor

            vim = {}

            ---@type vim.var_accessor
            vim.g = {}
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
            if vim.g.aaa then
                return
            end

            vim.g.aaa = true
            "#,
        ));
    }

    #[test]
    fn test_alias_unknown_allows_field_injection() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
                ---@alias Anything unknown

                ---@type Anything
                local a
                a.foo = 1
            "#
        ));
    }

    #[test]
    fn test_alias_object_and_unknown_table_initializers() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
                ---@alias Anything unknown

                ---@type Anything
                local a = { foo = 1 }
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::InjectField,
            r#"
                ---@alias Obj { x: number }

                ---@type Obj
                local a = { x = 1 }
            "#
        ));
    }
    #[test]
    fn test_partial_class_redeclaration_keeps_self_field() {
        let diagnostics = check_source(
            r#"
            ---@class (partial) Develop.AttrA
            local M = {}

            ---@class (partial) Develop.AttrA

            function M:__init()
                self.a = 1
            end
            "#,
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != DiagnosticCode::InjectField),
            "{diagnostics:?}"
        );
    }
    #[test]
    fn test_cross_file_redeclaration_keeps_self_field() {
        let meta = "---@meta

---@class Ability
";
        let good = r#"
            ---@class (partial) Ability
            local M = {}

            function M:__init(id)
                self.id = id
            end
            "#;
        let mut ws = VirtualWorkspace::new();
        ws.def_file("aaa_meta.lua", meta);
        let good_id = ws.def_file("zzz_good.lua", good);

        let inject_count = |ws: &VirtualWorkspace| {
            ws.analysis
                .diagnose_file(good_id, tokio_util::sync::CancellationToken::new())
                .unwrap()
                .into_iter()
                .filter(|d| {
                    matches!(&d.code, Some(lsp_types::NumberOrString::String(c)) if c == "inject-field")
                })
                .count()
        };
        assert_eq!(inject_count(&ws), 0);

        // 重算结果应保持一致.
        let uri = ws.analysis.get_uri(good_id).expect("uri");
        ws.analysis.update_file_by_uri(&uri, Some(good.to_string()));
        assert_eq!(inject_count(&ws), 0);
    }

    #[test]
    fn test_record_fields_distinguish_optional_and_required_values() {
        let source = r#"
            ---@param opts {label?: string, values?: table<string, string>}
            local function optional(opts) return opts end
            ---@type table<string, string>?
            local values
            optional({})
            optional({label = nil, values = nil})
            optional({values = values})
            optional({values = {name = 'ok'}})

            ---@param opts {label: string, values: table<string, string>}
            local function required(opts) return opts end
            required({label = 'ok', values = {}})
            required({label = nil, values = {}})
            required({label = 'ok', values = nil})
            "#;
        let diagnostics = check_source(source);
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            [
                DiagnosticCode::ParamTypeMismatch,
                DiagnosticCode::ParamTypeMismatch,
            ],
            "{source}\n{diagnostics:?}"
        );
    }
}

#[cfg(test)]
mod finite_union_alias_chains {
    use crate::DiagnosticCode;
    use crate::check::test::check_source;

    fn source(reverse: bool, statement: &str) -> String {
        let mut source = "---@alias A0 {value:number}\n".to_string();
        for alias in 1..=60 {
            let previous = alias - 1;
            let target = if reverse {
                format!("string|A{previous}")
            } else {
                format!("A{previous}|string")
            };
            source.push_str(&format!("---@alias A{alias} {target}\n"));
        }
        source.push_str(statement);
        source
    }

    fn contexts(value: &str) -> Vec<(String, DiagnosticCode)> {
        vec![
            (
                format!("---@type A60\nlocal assigned = {value}\nreturn assigned"),
                DiagnosticCode::AssignTypeMismatch,
            ),
            (
                format!(
                    "---@param value A60\nlocal function accept(value) return value end\naccept({value})"
                ),
                DiagnosticCode::ParamTypeMismatch,
            ),
            (
                format!(
                    "---@param ... A60\nlocal function accept(...) return ... end\naccept({value})"
                ),
                DiagnosticCode::ParamTypeMismatch,
            ),
            (
                format!(
                    "---@param ... [A60]...\nlocal function accept(...) return ... end\naccept({value})"
                ),
                DiagnosticCode::ParamTypeMismatch,
            ),
            (
                format!(
                    "---@param value A60\nlocal function accept(value) return value end\npcall(accept, {value})"
                ),
                DiagnosticCode::ParamTypeMismatch,
            ),
        ]
    }

    #[test]
    fn finite_alias_chains_accept_valid_constructors_in_every_context() {
        for reverse in [false, true] {
            for (statement, _) in contexts("{value=1}") {
                let diagnostics: Vec<_> = check_source(&source(reverse, &statement))
                    .into_iter()
                    .filter(|d| d.code != DiagnosticCode::Unused)
                    .collect();
                assert!(
                    diagnostics.is_empty(),
                    "reverse={reverse}: {statement}: {diagnostics:?}"
                );
            }
        }
    }

    #[test]
    fn finite_alias_chains_reject_invalid_constructors_in_every_context() {
        for reverse in [false, true] {
            for (statement, expected) in contexts("{value='bad'}") {
                let diagnostics = check_source(&source(reverse, &statement));
                assert!(
                    diagnostics.iter().any(|d| d.code == expected),
                    "reverse={reverse}: {statement}: {diagnostics:?}"
                );
            }
        }
    }
}

// These tests load the standard library because its `table` declaration is
// what made dictionaries incorrectly look like closed records.

mod dictionary_constructor {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use googletest::prelude::*;

    #[googletest::gtest]
    fn dictionary_constructor_accepts_keys_with_standard_library() {
        for declaration in ["local signals", "signals"] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            let source = format!(
                r#"
            ---@class Signal
            ---@field name string
            KEY = 'depot'
            ---@type table<string, Signal>
            {declaration} = {{
                [KEY] = {{name = KEY}},
                literal = {{name = 'literal'}},
                ['bracket'] = {{name = 'bracket'}},
            }}
        "#
            );
            assert_that!(
                ws.has_no_diagnostic(DiagnosticCode::InjectField, &source),
                eq(true),
                "{declaration}"
            );
        }
    }

    #[googletest::gtest]
    fn dictionary_constructor_alias_and_call_argument() {
        for target in ["table<integer, string>", "Dictionary"] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def("---@alias Dictionary table<integer, string>");
            let source = format!(
                r#"
            ---@param values {target}
            local function consume(values) end
            consume({{[1] = 'one', [2] = 'two'}})
            ---@type {target}
            local values = {{}}
            values = {{[3] = 'three'}}
        "#
            );
            assert_that!(
                ws.has_no_diagnostic(DiagnosticCode::InjectField, &source),
                eq(true),
                "{target}"
            );
        }
    }

    #[googletest::gtest]
    fn dictionary_constructor_does_not_disable_class_field_checks() {
        for target in ["Record", "Box<string>"] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(
                r#"
            ---@class Record
            ---@field name string
            ---@class Box<T>
            ---@field name T
        "#,
            );
            assert_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::InjectField,
                    &format!("---@type {target}\nlocal value = {{name = 'ok', extra = true}}")
                ),
                eq(false),
                "{target}"
            );
            assert_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::MissingFields,
                    &format!("---@type {target}\nlocal value = {{}}")
                ),
                eq(false),
                "{target}"
            );
        }
    }

    #[googletest::gtest]
    fn dictionary_constructor_preserves_user_defined_table_types() {
        for private in [false, true] {
            let mut ws = if private {
                VirtualWorkspace::new_with_init_std_lib()
            } else {
                VirtualWorkspace::new()
            };
            let modifier = if private { "(private) " } else { "" };
            let source = format!(
                "---@class {modifier}table<T>\n---@field name T\n---@type table<string>\nlocal value = {{name = 'ok', extra = true}}"
            );
            // has_no_diagnostic enables and matches exactly the requested code.
            assert_that!(
                ws.has_no_diagnostic(DiagnosticCode::InjectField, &source),
                eq(false),
                "{modifier}"
            );
        }
    }

    #[googletest::gtest]
    fn dictionary_constructor_compound_targets_keep_record_checks() {
        for target in [
            "table<string, integer> & Record",
            "Record & table<string, integer>",
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(&format!(
                "---@class Record\n---@field name string\n---@alias Combined {target}"
            ));
            assert_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::MissingFields,
                    "---@type Combined\nlocal value = {}"
                ),
                eq(false),
                "{target}"
            );
            assert_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::InjectField,
                    "---@type Combined\nlocal value = {name = 'ok', extra = true}"
                ),
                eq(false),
                "{target}"
            );
        }
        for target in [
            "table<string, integer> | nil",
            "nil | table<string, integer>",
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            assert_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::InjectField,
                    &format!("---@type {target}\nlocal value = {{answer = 42}}")
                ),
                eq(true),
                "{target}"
            );
        }
    }
}

#[cfg(test)]
mod union_table_constructors {
    use crate::DiagnosticCode;
    use crate::check::test::check_source;

    const TYPES: &str = r#"
        ---@class Record
        ---@field valid boolean
        ---@field members integer[]
        local record = {}
        ---@class Named
        ---@field id integer
        ---@field name string
        local named = {}
        ---@class Enabled
        ---@field id string
        ---@field enabled boolean
        local enabled = {}
    "#;

    fn diagnostics(target: &str, expression: &str) -> Vec<crate::check::checker::Diagnostic> {
        let source = format!(
            r#"
            {TYPES}
            ---@alias Choice {target}
            ---@param value Choice
            local function accept(value) return value end
            accept({expression})
            ---@type Choice
            local assigned = {expression}
            return assigned
        "#
        );
        check_source(&source)
            .into_iter()
            .filter(|d| d.code != DiagnosticCode::Unused)
            .collect()
    }

    #[test]
    fn accepts_complete_alternatives_independent_of_order() {
        for (target, expression) in [
            ("[string, string]|Record|string", "{'message', 'argument'}"),
            ("Record|string|[string, string]", "{'message', 'argument'}"),
            ("Record|{x:number,y:number}", "{x=1,y=2}"),
            ("string|{x:number,y:number}", "{x=1,y=2}"),
            ("Record|string[]", "{'one', 'two'}"),
            ("Named|Enabled", "{id='id', enabled=true}"),
            ("Enabled|Named", "{id=1, name='name'}"),
            ("Record|table", "{arbitrary=true}"),
            ("Record|any", "{arbitrary=true}"),
            ("Record|unknown", "{arbitrary=true}"),
            ("{nested?:{left:number}}|{name:string}", "{}"),
            (
                "{nested:{left:number}}|{nested:{right:string}}",
                "{nested={}, [('nested')]={left=1}}",
            ),
            (
                "{nested:{left:number}}|{nested:{right:string}}",
                "{[('nested')]={left=1}}",
            ),
            (
                "{nested: {left:number}|{right:string}}|{name:string}",
                "{nested={right='text'}}",
            ),
            (
                "{nested:{left:number}}|{nested:{right:string}}",
                "{nested={right='text'}}",
            ),
        ] {
            let diagnostics = diagnostics(target, expression);
            assert!(diagnostics.is_empty(), "{target}: {diagnostics:?}");
        }
    }

    #[test]
    fn rejects_incomplete_or_mistyped_constructors() {
        for (target, expression) in [
            ("[string, string]|Record", "{1, 'argument'}"),
            ("[string, string]|Record", "{'message'}"),
            ("Record|string[]", "{function() end}"),
            ("Named|Enabled", "{id='id', name='name'}"),
            ("Enabled|Named", "{id='id', name='name'}"),
            ("Named|Enabled", "{id=1}"),
            ("Named|Named[]", "{id=1}"),
            ("Record|{x:number,y:number}", "{x='wrong',y=2}"),
            ("string|{x:number,y:number}", "{x='wrong',y=2}"),
            ("{nested?:{left:number}}|{name:string}", "{nested={}}"),
            (
                "{nested:{left:number}}|{nested:{right:string}}",
                "{nested={left=1}, [('nested')]={}}",
            ),
            (
                "{nested: {left:number}|{right:string}}|{name:string}",
                "{nested={left=false}}",
            ),
            (
                "{nested:{left:number}}|{nested:{right:string}}",
                "{nested={left=false}}",
            ),
            (
                "{nested:{left:number}}|{nested:{right:string}}",
                "{nested={}}",
            ),
        ] {
            let diagnostics = diagnostics(target, expression);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
                "{target}: {diagnostics:?}"
            );
            assert!(
                diagnostics.iter().any(|d| matches!(
                    d.code,
                    DiagnosticCode::AssignTypeMismatch
                        | DiagnosticCode::MissingFields
                        | DiagnosticCode::InjectField
                )),
                "{target}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn follows_aliases_without_requiring_every_branch() {
        let diagnostics = check_source(&format!(
            r#"
            {TYPES}
            ---@alias Pair [string, string]
            ---@alias PairAlias Pair
            ---@alias Choice PairAlias|Record
            ---@param value Choice
            local function accept(value) return value end
            accept({{'message', 'argument'}})
        "#
        ));
        assert!(
            diagnostics.iter().all(|d| d.code == DiagnosticCode::Unused),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn diagnostic_filters_do_not_select_an_incompatible_branch() {
        let diagnostics = check_source(&format!(
            r#"
            {TYPES}
            ---@alias Choice Named|Enabled
            ---@param value Choice
            local function accept(value) return value end
            ---@diagnostic disable: assign-type-mismatch
            accept({{id='id', name='name'}})
        "#
        ));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
            "{diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::AssignTypeMismatch),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn recursive_union_aliases_terminate_and_keep_matching_branches() {
        let diagnostics = check_source(
            r#"
            ---@alias Recursive Recursive|[string, string]
            ---@param value Recursive
            local function accept(value) return value end
            accept({'message', 'argument'})
        "#,
        );
        assert!(
            diagnostics.iter().all(|d| d.code == DiagnosticCode::Unused),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn direct_and_protected_calls_keep_selected_union_field_mismatches() {
        for callee in ["accept(", "pcall(accept, "] {
            for (fields, value, valid) in [
                ("{value: number}|{value: string}", "{value=2}", true),
                ("{value: number}|{value: string}", "{value='text'}", true),
                ("{value: number}|{value: string}", "{value=false}", false),
                (
                    "{[string]: number}|{[string]: string}",
                    "{value=false}",
                    false,
                ),
                (
                    "{a: number,b: number}|{a: string,b: string}",
                    "{a=2,b='text'}",
                    false,
                ),
            ] {
                let source = format!(
                    "---@alias Fields {fields}\n---@param value Fields\nlocal function accept(value) return value end\n{callee}{value})"
                );
                let diagnostics = check_source(&source);
                assert_eq!(diagnostics.is_empty(), valid, "{source}: {diagnostics:?}");
                if !valid {
                    assert!(
                        diagnostics.iter().any(|diagnostic| {
                            diagnostic.code == DiagnosticCode::AssignTypeMismatch
                        }),
                        "{source}: {diagnostics:?}"
                    );
                    assert!(
                        diagnostics.iter().any(|diagnostic| {
                            diagnostic.code == DiagnosticCode::ParamTypeMismatch
                        }),
                        "{source}: {diagnostics:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn array_only_unions_reject_named_fields() {
        for (target, expression) in [
            ("integer[]|string[]", "{id=1}"),
            ("integer[]|string[]", "{[('id')]=1}"),
            ("[string,string]|[number]", "{id=1}"),
        ] {
            let diagnostics = diagnostics(target, expression);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
                "{target} / {expression}: {diagnostics:?}"
            );
            assert!(
                diagnostics.iter().any(|d| matches!(
                    d.code,
                    DiagnosticCode::AssignTypeMismatch
                        | DiagnosticCode::InjectField
                        | DiagnosticCode::MissingFields
                )),
                "{target} / {expression}: {diagnostics:?}"
            );
        }
        for expression in ["{1,2}", "{'one','two'}", "{}"] {
            let diagnostics = diagnostics("integer[]|string[]", expression);
            assert!(diagnostics.is_empty(), "{expression}: {diagnostics:?}");
        }
    }

    #[test]
    fn recursive_intersection_union_uses_concrete_alternative() {
        for (expression, valid) in [
            ("{next={name='leaf'}}", true),
            ("{next={next={name='leaf'}}}", true),
            ("{next={name=1}}", false),
        ] {
            let source = format!(
                "---@alias Chain {{next:Chain}}|{{name:string}}\n---@param value Chain\nlocal function accept(value) return value end\naccept({expression})"
            );
            let diagnostics = check_source(&source);
            assert_eq!(
                diagnostics.is_empty(),
                valid,
                "{expression}: {diagnostics:?}"
            );
        }
        for expression in ["{name='ok'}", "{value='bad'}", "{value=1}"] {
            let source = format!(
                "---@alias Recursive (Recursive & {{value:number}})|{{name:string}}\n---@param value Recursive\nlocal function accept(value) return value end\naccept({expression})"
            );
            let diagnostics = check_source(&source);
            assert_eq!(
                diagnostics.is_empty(),
                expression == "{name='ok'}",
                "{expression}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn union_selection_does_not_switch_for_disabled_codes() {
        let source = r#"
            ---@class First
            ---@field x number
            local first = {}
            ---@class Second
            ---@field x string
            ---@field y string
            local second = {}
            ---@type First|Second
            local value = {x=false}
            return value
        "#;
        let diagnostics: Vec<_> = check_source(source)
            .into_iter()
            .filter(|d| d.code != DiagnosticCode::Unused)
            .collect();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].code, DiagnosticCode::AssignTypeMismatch);
        let mut config = crate::Emmyrc::default();
        config
            .diagnostics
            .disable
            .push(DiagnosticCode::AssignTypeMismatch);
        let diagnostics = crate::check::test::check_source_with_emmyrc(source, config);
        assert!(
            diagnostics.iter().all(|d| d.code == DiagnosticCode::Unused),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn union_diagnostics_use_active_severity_and_keep_distinct_fields() {
        let source = r#"
            ---@class First
            ---@field x number
            local first = {}
            ---@class Second
            ---@field y string
            local second = {}
            ---@type First|Second
            local value = {extra=1, other=2}
            return value
        "#;
        let mut config = crate::Emmyrc::default();
        config.diagnostics.severity.insert(
            DiagnosticCode::InjectField,
            crate::DiagnosticSeveritySetting::Error,
        );
        config.diagnostics.severity.insert(
            DiagnosticCode::MissingFields,
            crate::DiagnosticSeveritySetting::Hint,
        );
        let diagnostics: Vec<_> = crate::check::test::check_source_with_emmyrc(source, config)
            .into_iter()
            .filter(|d| d.code != DiagnosticCode::Unused)
            .collect();
        assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
        let injected: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::InjectField)
            .collect();
        assert_eq!(injected.len(), 2, "{diagnostics:?}");
        assert_eq!(injected[0].range, injected[1].range);
        assert_ne!(injected[0].message, injected[1].message);
        assert!(
            injected
                .iter()
                .all(|d| d.severity == lsp_types::DiagnosticSeverity::ERROR)
        );
        let missing = diagnostics
            .iter()
            .find(|d| d.code == DiagnosticCode::MissingFields)
            .unwrap();
        assert_eq!(missing.severity, lsp_types::DiagnosticSeverity::HINT);
    }

    #[test]
    fn concrete_recursive_result_can_satisfy_an_outer_alternative() {
        for (expression, valid) in [("{nested={name='ok'}}", true), ("{nested={name=1}}", false)] {
            let source = format!(
                "---@alias Recursive (Recursive & {{value:number}})|{{name:string}}\n---@alias Outer {{nested:Recursive}}|{{other:boolean}}\n---@param value Outer\nlocal function accept(value) return value end\naccept({expression})"
            );
            let diagnostics = check_source(&source);
            assert_eq!(
                diagnostics.is_empty(),
                valid,
                "{expression}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn unresolved_recursive_union_is_not_compatible() {
        for expression in ["{}", "{value=1,name='ok'}"] {
            let source = format!(
                "---@alias Recursive (Recursive & {{value:number}})|(Recursive & {{name:string}})\n---@param value Recursive\nlocal function accept(value) return value end\naccept({expression})\n---@type Recursive\nlocal assigned = {expression}\nreturn assigned"
            );
            let diagnostics = check_source(&source);
            for code in [
                DiagnosticCode::ParamTypeMismatch,
                DiagnosticCode::AssignTypeMismatch,
            ] {
                assert!(
                    diagnostics.iter().any(|d| d.code == code),
                    "{expression}: {diagnostics:?}"
                );
            }
        }
    }

    #[test]
    fn whole_constructor_mismatch_is_reported_once_per_context() {
        for (statement, expected) in [
            (
                "---@type Choice\nlocal assigned = {}\nreturn assigned",
                DiagnosticCode::AssignTypeMismatch,
            ),
            ("accept({})", DiagnosticCode::ParamTypeMismatch),
            ("pcall(accept, {})", DiagnosticCode::ParamTypeMismatch),
        ] {
            let source = format!(
                "---@alias Choice string|{{x:number,y:number}}\n---@param value Choice\nlocal function accept(value) return value end\n{statement}"
            );
            let diagnostics: Vec<_> = check_source(&source)
                .into_iter()
                .filter(|d| d.code != DiagnosticCode::Unused)
                .collect();
            assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
            assert_eq!(diagnostics[0].code, expected, "{source}: {diagnostics:?}");
            assert_eq!(diagnostics[0].range.len(), rowan::TextSize::from(2));
        }
    }

    #[test]
    fn union_aliases_resolve_in_the_defining_file() {
        for target in ["Shared.Raw", "Shared.Choice", "Choice"] {
            for (expression, valid) in [("{x=1}", true), ("{y='ok'}", true), ("{x=false}", false)] {
                let mut ws = crate::VirtualWorkspace::new();
                ws.def_file("types.lua", "---@namespace Shared\n---@alias Raw {x:number}|{y:string}\n---@alias Choice Raw\nreturn {}");
                let source = format!(
                    "---@using Shared\n---@param value {target}\nlocal function accept(value) return value end\naccept({expression})\n---@type {target}\nlocal assigned = {expression}\nreturn assigned"
                );
                for code in [
                    DiagnosticCode::ParamTypeMismatch,
                    DiagnosticCode::AssignTypeMismatch,
                ] {
                    assert_eq!(
                        ws.has_no_diagnostic(code, &source),
                        valid,
                        "{target}: {expression}: {code:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn equal_failure_counts_keep_declared_alternative_order() {
        for (target, expected) in [
            ("{value:number}|{value:string}", "number"),
            ("{value:string}|{value:number}", "string"),
        ] {
            let source = format!(
                "---@alias Choice {target}\n---@type Choice\nlocal assigned = {{value=false}}\nreturn assigned"
            );
            for _ in 0..2 {
                let diagnostics: Vec<_> = check_source(&source)
                    .into_iter()
                    .filter(|d| d.code != DiagnosticCode::Unused)
                    .collect();
                assert_eq!(diagnostics.len(), 1, "{target}: {diagnostics:?}");
                assert_eq!(diagnostics[0].code, DiagnosticCode::AssignTypeMismatch);
                assert!(
                    diagnostics[0].message.contains(expected),
                    "{target}: {diagnostics:?}"
                );
            }
        }
    }

    #[test]
    fn union_arguments_report_structural_failure_with_only_parameter_warnings_enabled() {
        for signature in [
            "---@param value Choice\nlocal function accept(value) return value end",
            "---@param ... Choice\nlocal function accept(...) return ... end",
            "---@param ... [Choice]...\nlocal function accept(...) return ... end",
        ] {
            for (argument, valid) in [("{left=1}", true), ("{}", false), ("{left=false}", false)] {
                let source = format!(
                    "---@alias Choice {{left:number}}|{{right:string}}\n{signature}\naccept({argument})"
                );
                let mut ws = crate::VirtualWorkspace::new();
                assert_eq!(
                    ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, &source),
                    valid,
                    "{source}"
                );
            }
        }
    }
    #[test]
    fn union_object_keys_distinguish_integer_and_string_literals() {
        for (target, value, valid) in [
            ("{[1]:number}|{name:string}", "{[1]=7}", true),
            ("{[1]:number}|{name:string}", "{[(1)]=7}", true),
            ("{[1]:number}|{name:string}", "{[1.0]=7}", true),
            ("{[1]:number}|{name:string}", "{[(1.0)]=7}", true),
            ("{[1]:number}|{name:string}", "{[(1)]='bad'}", false),
            ("{[1]:number}|{name:string}", "{[1.0]='bad'}", false),
            ("{[1]:number}|{name:string}", "{[(1.0)]='bad'}", false),
            ("{[1]:number}|{name:string}", "{[1.5]=7}", false),
            ("{[1]:number}|{name:string}", "{7}", true),
            ("{[1]:number}|{name:string}", "{['1']=7}", false),
            ("{['1']:number}|{name:string}", "{['1']=7}", true),
            ("{['1']:number}|{name:string}", "{[('1')]=7}", true),
            ("{['1']:number}|{name:string}", "{[('1')]='bad'}", false),
            ("{['1']:number}|{name:string}", "{[1]=7}", false),
        ] {
            let diagnostics = diagnostics(target, value);
            assert_eq!(
                diagnostics.is_empty(),
                valid,
                "{target} / {value}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn union_object_scalar_values_keep_assignment_failures() {
        for statement in [
            "---@type Choice\nlocal value = {nested=5}\nreturn value",
            "---@param value Choice\nlocal function accept(value) return value end\naccept({nested=5})",
            "---@param value Choice\nlocal function accept(value) return value end\npcall(accept, {nested=5})",
        ] {
            let source = format!(
                "---@alias Choice {{nested:{{value:number}}}}|{{nested:{{name:string}}}}\n{statement}"
            );
            let diagnostics = check_source(&source);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::AssignTypeMismatch),
                "{source}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn isolated_union_checkers_keep_structural_and_value_ownership() {
        use crate::check::CheckConfig;
        use crate::check::checker::{
            CheckContext, Checker, assign_type_mismatch::AssignTypeMismatchChecker,
            check_field::CheckFieldChecker,
        };
        use std::sync::Arc;
        for (value, expected_structural, expected_value) in
            [("{}", true, false), ("{nested=5}", false, true)]
        {
            for field_first in [false, true] {
                let mut ws = crate::VirtualWorkspace::new();
                let source = format!(
                    "---@alias Choice {{nested:{{value:number}}}}|{{nested:{{name:string}}}}\n---@type Choice\nlocal value = {value}\nreturn value"
                );
                let file = ws.def(&source);
                let model = ws.analysis.semantic_model(file);
                let mut fields = CheckContext::new(&model, Arc::new(CheckConfig::default()));
                let mut values = CheckContext::new(&model, Arc::new(CheckConfig::default()));
                if field_first {
                    CheckFieldChecker::check(&mut fields, &model);
                    AssignTypeMismatchChecker::check(&mut values, &model);
                } else {
                    AssignTypeMismatchChecker::check(&mut values, &model);
                    CheckFieldChecker::check(&mut fields, &model);
                }
                let fields = fields.into_diagnostics();
                let values = values.into_diagnostics();
                assert_eq!(
                    !fields.is_empty(),
                    expected_structural,
                    "{source}: {fields:?}"
                );
                assert_eq!(!values.is_empty(), expected_value, "{source}: {values:?}");
                assert!(fields.iter().all(|d| matches!(
                    d.code,
                    DiagnosticCode::MissingFields | DiagnosticCode::InjectField
                )));
                assert!(
                    values
                        .iter()
                        .all(|d| d.code == DiagnosticCode::AssignTypeMismatch)
                );
            }
        }
    }

    #[test]
    fn union_computed_object_keys_check_value_types() {
        for (target, value) in [
            ("{[1]:number}|{name:string}", "{[(1)]='bad'}"),
            ("{[1]:number}|{name:string}", "{[1.0]='bad'}"),
            ("{['1']:number}|{name:string}", "{[('1')]='bad'}"),
        ] {
            let diagnostics = diagnostics(target, value);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::AssignTypeMismatch),
                "{target} / {value}: {diagnostics:?}"
            );
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
                "{target} / {value}: {diagnostics:?}"
            );
        }
    }
}

#[cfg(test)]
mod constructor_depth_tests {
    use crate::{DiagnosticCode, check::test::check_source};

    #[test]
    fn scalar_only_union_reports_once_in_each_context() {
        for target in ["string|number", "number|string"] {
            for (statement, expected) in [
                (
                    "---@type Choice\nlocal value = {1}\nreturn value",
                    DiagnosticCode::AssignTypeMismatch,
                ),
                (
                    "---@param value Choice\nlocal function accept(value) return value end\naccept({1})",
                    DiagnosticCode::ParamTypeMismatch,
                ),
                (
                    "---@param value Choice\nlocal function accept(value) return value end\npcall(accept, {1})",
                    DiagnosticCode::ParamTypeMismatch,
                ),
                (
                    "---@param ... Choice\nlocal function accept(...) return ... end\naccept({1})",
                    DiagnosticCode::ParamTypeMismatch,
                ),
                (
                    "---@param ... [Choice]...\nlocal function accept(...) return ... end\naccept({1})",
                    DiagnosticCode::ParamTypeMismatch,
                ),
            ] {
                let source = format!("---@alias Choice {target}\n{statement}");
                let diagnostics = check_source(&source)
                    .into_iter()
                    .filter(|d| d.code != DiagnosticCode::Unused)
                    .collect::<Vec<_>>();
                assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
                assert_eq!(diagnostics[0].code, expected, "{source}: {diagnostics:?}");
                assert_eq!(diagnostics[0].range.len(), rowan::TextSize::from(3));
            }
        }
    }

    #[test]
    fn mutual_alias_cycles_keep_the_concrete_exit() {
        for alternative in ["A|{x:number}", "{x:number}|A"] {
            for (value, valid) in [("{x=1}", true), ("{x='bad'}", false)] {
                let source = format!(
                    "---@alias A B\n---@alias B {alternative}\n---@param value A\nlocal function accept(value) return value end\naccept({value})"
                );
                let diagnostics = check_source(&source)
                    .into_iter()
                    .filter(|d| d.code != DiagnosticCode::Unused)
                    .collect::<Vec<_>>();
                assert_eq!(diagnostics.is_empty(), valid, "{source}: {diagnostics:?}");
            }
        }
    }

    fn isolated_checks(source: String) -> Vec<crate::check::checker::Diagnostic> {
        use crate::check::{
            CheckConfig,
            checker::{
                CheckContext, Checker, assign_type_mismatch::AssignTypeMismatchChecker,
                check_field::CheckFieldChecker, param_type_check::ParamTypeChecker,
            },
        };
        // File loading/dropping has a separate inherited native-stack limit.
        // Only those stages use this helper; every checker runs on the test thread.
        let (ws, file) = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let mut ws = crate::VirtualWorkspace::new();
                let file = ws.def(&source);
                (ws, file)
            })
            .unwrap()
            .join()
            .unwrap();
        let mut diagnostics = Vec::new();
        for checker in 0..3 {
            // A fresh model prevents a previous checker's cache from hiding recursion.
            let model = ws.analysis.semantic_model(file);
            let mut context =
                CheckContext::new(&model, std::sync::Arc::new(CheckConfig::default()));
            match checker {
                0 => CheckFieldChecker::check(&mut context, &model),
                1 => AssignTypeMismatchChecker::check(&mut context, &model),
                _ => ParamTypeChecker::check(&mut context, &model),
            }
            diagnostics.extend(context.into_diagnostics());
        }
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || drop(ws))
            .unwrap()
            .join()
            .unwrap();
        diagnostics
    }

    #[test]
    fn deep_object_constructors_keep_field_and_value_failures() {
        for depth in [256, 512] {
            for (leaf, valid) in [("{value=1}", true), ("{value='bad'}", false), ("{}", false)] {
                let mut value = leaf.to_string();
                for _ in 0..depth {
                    value = format!("{{value=1,next={value}}}");
                }
                for (statement, expected) in [
                    (
                        format!("---@type Node|string\nlocal value = {value}\nreturn value"),
                        DiagnosticCode::AssignTypeMismatch,
                    ),
                    (
                        format!(
                            "---@param value Node|string\nlocal function accept(value) return value end\naccept({value})"
                        ),
                        DiagnosticCode::ParamTypeMismatch,
                    ),
                    (
                        format!(
                            "---@param value Node|string\nlocal function accept(value) return value end\npcall(accept, {value})"
                        ),
                        DiagnosticCode::ParamTypeMismatch,
                    ),
                    (
                        format!(
                            "---@param ... Node|string\nlocal function accept(...) return ... end\naccept({value})"
                        ),
                        DiagnosticCode::ParamTypeMismatch,
                    ),
                    (
                        format!(
                            "---@param ... [Node|string]...\nlocal function accept(...) return ... end\naccept({value})"
                        ),
                        DiagnosticCode::ParamTypeMismatch,
                    ),
                ] {
                    let source = format!("---@alias Node {{value:number,next:Node?}}\n{statement}");
                    let diagnostics = isolated_checks(source);
                    assert_eq!(
                        diagnostics.is_empty(),
                        valid,
                        "depth={depth}, leaf={leaf}: {diagnostics:?}"
                    );
                    if !valid {
                        assert!(
                            diagnostics.iter().any(|d| d.code == expected),
                            "depth={depth}, leaf={leaf}: {diagnostics:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn deep_array_and_index_constructors_keep_leaf_failures() {
        for (definition, array) in [
            ("---@alias Node {value:number,children:Node[]?}", true),
            (
                "---@class Node\n---@field value number\n---@field [integer] Node",
                false,
            ),
        ] {
            for (leaf, valid) in [("{value=1}", true), ("{value='bad'}", false), ("{}", false)] {
                let mut value = leaf.to_string();
                for _ in 0..128 {
                    value = if array {
                        format!("{{value=1,children={{{value}}}}}")
                    } else {
                        format!("{{value=1,[1]={value}}}")
                    };
                }
                let source = format!(
                    "{definition}\n---@param value Node|string\nlocal function accept(value) return value end\naccept({value})"
                );
                let diagnostics = isolated_checks(source);
                assert_eq!(
                    diagnostics.is_empty(),
                    valid,
                    "array={array}, leaf={leaf}: {diagnostics:?}"
                );
                if !valid {
                    assert!(
                        diagnostics
                            .iter()
                            .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
                        "array={array}, leaf={leaf}: {diagnostics:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn deep_recursive_union_keeps_a_successful_sibling() {
        for (leaf, valid) in [("{text='ok'}", true), ("{text=1}", false)] {
            let mut value = leaf.to_string();
            for _ in 0..128 {
                value = format!("{{value=1,next={value}}}");
            }
            let source = format!(
                "---@alias Node {{value:number,next:Node?}}|{{text:string,next:Node?}}\n---@param value Node\nlocal function accept(value) return value end\naccept({value})"
            );
            let diagnostics = isolated_checks(source);
            assert_eq!(
                diagnostics.is_empty(),
                valid,
                "leaf={leaf}: {diagnostics:?}"
            );
            if !valid {
                assert!(
                    diagnostics
                        .iter()
                        .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
                    "{diagnostics:?}"
                );
            }
        }
    }
    #[test]
    fn deep_named_index_fields_report_a_missing_leaf() {
        for (leaf, valid) in [("{value=1}", true), ("{}", false)] {
            let mut value = leaf.to_string();
            for _ in 0..256 {
                value = format!("{{value=1,[1]={value}}}");
            }
            let source = format!(
                "---@class Node\n---@field value number\n---@field [integer] Node\n---@type Node\nlocal value = {value}\nreturn value"
            );
            let diagnostics = isolated_checks(source);
            assert_eq!(
                diagnostics.is_empty(),
                valid,
                "leaf={leaf}: {diagnostics:?}"
            );
            if !valid {
                assert!(
                    diagnostics
                        .iter()
                        .any(|d| d.code == DiagnosticCode::MissingFields),
                    "{diagnostics:?}"
                );
            }
        }
    }
}
