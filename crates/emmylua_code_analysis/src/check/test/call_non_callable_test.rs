#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use lsp_types::NumberOrString;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn test_callable_table_accepts_valid_arguments() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file_id = ws.def(
            r#"
            local command = setmetatable({}, {
                ---@param arg string
                __call = function(_, arg) print(arg) end,
            })
            command('hello')
            local api = { cmd = command }
            api.cmd('hello')
            "#,
        );
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn test_callable_table_preserves_invalid_call_diagnostics() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file_id = ws.def(
            r#"
            local command = setmetatable({}, {
                ---@param arg string
                __call = function(_, arg) print(arg) end,
            })
            command(42)
            local plain = {}
            plain()
            local without_call = setmetatable({}, {})
            without_call()
            "#,
        );
        let mut diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
        let codes: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect();
        assert_eq!(
            codes,
            vec![
                Some(NumberOrString::String("param-type-mismatch".into())),
                Some(NumberOrString::String("call-non-callable".into())),
                Some(NumberOrString::String("call-non-callable".into())),
            ],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn test_callable_table_member_assignment_across_files() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "api.lua",
            r#"
            local api = {}
            api.cmd = setmetatable({}, {
                ---@param arg string
                __call = function(_, arg) print(arg) end,
            })
            api.plain = {}
            return api
            "#,
        );
        let file_id = ws.def(
            r#"
            local api = require('api')
            api.cmd('hello')
            api.cmd(42)
            api.plain()
            "#,
        );
        let mut diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
        let codes: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect();
        assert_eq!(
            codes,
            vec![
                Some(NumberOrString::String("param-type-mismatch".into())),
                Some(NumberOrString::String("call-non-callable".into())),
            ],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn test_callable_class_overload_from_another_file() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "merger.lua",
            r#"
            ---@namespace example
            ---@class Merger
            ---@overload fun<T>(target:T):T
            local M = {}
            return M
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@type example.Merger
            local merge = require('merger')
            local result = merge('hello')
            merge('again')
            "#,
        ));
    }

    #[test]
    fn test_callable_class_overload_aliases_and_generics() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@class Callable<T>
            ---@overload fun(value:T):T
            ---@alias CallableAlias Callable<string>
            ---@alias GenericCallableAlias<T> Callable<T>
            ---@alias FunctionAlias<T> fun(value:T):T
            ---@type Callable<string>
            local direct
            ---@type CallableAlias
            local alias
            ---@type GenericCallableAlias<string>
            local generic_alias
            ---@type FunctionAlias<string>
            local function_alias
            direct('hello')
            alias('hello')
            generic_alias('hello')
            function_alias('hello')
            "#,
        ));
    }

    #[test]
    fn test_callable_class_overload_unions_and_constraints() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@class Callable
            ---@overload fun():string
            ---@type Callable|fun():string
            local either
            ---@type Callable?
            local optional
            either()
            optional()
            ---@generic T:Callable
            ---@param callback T
            local function invoke(callback)
                callback()
            end
            "#,
        ));
    }

    #[test]
    fn test_callable_class_overload_preserves_non_callable_types() {
        let mut ws = VirtualWorkspace::new();
        let file_id = ws.def(
            r#"
            ---@class Callable
            ---@overload fun():string
            ---@class Plain<T>
            ---@class Child:Callable
            ---@type Callable|integer
            local mixed
            ---@type Plain<string>
            local plain
            ---@type Child
            local child
            mixed()
            plain()
            child()
            local table = {}
            table()
            "#,
        );
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        let non_callable: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("call-non-callable".into()))
            })
            .collect();
        assert_eq!(non_callable.len(), 4, "{diagnostics:#?}");
    }

    #[test]
    fn test_callable_class_overload_keeps_argument_errors() {
        let mut ws = VirtualWorkspace::new();
        let file_id = ws.def(
            r#"
            ---@class Callable
            ---@overload fun(value:string):string
            ---@type Callable
            local callback
            callback(42)
            "#,
        );
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        let codes: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect();
        assert_eq!(
            codes,
            vec![Some(NumberOrString::String("param-type-mismatch".into()))],
            "{diagnostics:#?}",
        );
    }

    #[test]
    fn test_callable_class_overload_recursive_alias_is_not_callable() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@alias Recursive<T> Recursive<T>
            ---@type Recursive<string>
            local callback
            callback()
            "#,
        ));
    }

    #[test]
    fn test_call_non_callable() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local i = 1
                i()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local s = "hi"
                s()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local b = true
                b()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type thread
                local t
                t()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local function f() end
                f()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type function
                local f
                f()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type function|integer
                local f
                f()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local f ---@type { field: string } & fun()
                f()
            "#
        ));

        // nil is covered by need-check-nil instead of call-non-callable
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type function|nil
                local f
                f()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                (1)()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ("hi")()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                (true)()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                (function() end)()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local i --- @type integer|fun():string
                _ = i()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@class Callable
                ---@operator call: string
                ---@type Callable
                local c
                c()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local c = {}
                c()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@class NonCallable
                ---@type NonCallable
                local c = {}
                c()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type any
                local c
                c()
            "#
        ));
    }

    #[test]
    fn test_call_non_callable_fallback_from_initializer() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_check(DiagnosticCode::CallNonCallable);
        let file_id = ws.def("local i = 1\n i()\n");

        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        let code = Some(NumberOrString::String(
            DiagnosticCode::CallNonCallable.get_name().to_string(),
        ));
        assert!(diagnostics.iter().any(|diag| diag.code == code));
    }

    #[test]
    fn test_no_call_non_callable_on_undefined_type() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            local a --- @type NotDefined
            a()
            "#
        ));
    }

    #[test]
    fn test_no_call_non_callable_on_alias() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@alias FunAlias function
            local a --- @type FunAlias
            a()
            "#
        ));
    }

    #[test]
    fn test_no_call_non_callable_for_generic_function_param_in_a_lua() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            --- @generic F: function
            --- @param fn F
            --- @return F
            function once(fn)
              return function(...)
                return fn(...)
              end
            end
            "#,
        ));
    }
}
