#[cfg(test)]
mod inline_assertion_arguments {
    use crate::DiagnosticCode;
    use crate::check::test::check_source;

    fn diagnostics(source: &str) -> Vec<crate::check::checker::Diagnostic> {
        check_source(source)
            .into_iter()
            .filter(|d| {
                matches!(
                    d.code,
                    DiagnosticCode::ParamTypeMismatch
                        | DiagnosticCode::AssignTypeMismatch
                        | DiagnosticCode::MissingFields
                        | DiagnosticCode::InjectField
                )
            })
            .collect()
    }

    #[test]
    fn test_asserted_table_arguments_use_asserted_contract() {
        let source = r#"
        ---@class Message
        ---@field text string
        ---@param message Message
        local function accept(message) end
        ---@type string?
        local maybe
        accept({text = maybe} --[[@as Message]])
        accept({unexpected = true} --[[@as Message]])
        accept(({} --[[@as Message]]))
        accept(({}) --[[@as Message]])
        "#;
        assert!(diagnostics(source).is_empty(), "{:#?}", diagnostics(source));
    }

    #[test]
    fn test_assertions_do_not_hide_other_argument_errors() {
        let source = r#"
        ---@class Message
        ---@field text string
        ---@param message Message
        ---@param count integer
        local function accept(message, count) end
        accept({} --[[@as Message]], 'wrong')
        "#;
        let result = diagnostics(source);
        assert_eq!(result.len(), 1, "{result:#?}");
        assert_eq!(result[0].code, DiagnosticCode::ParamTypeMismatch);
        assert!(result[0].message.contains("integer"), "{result:#?}");
    }

    #[test]
    fn test_asserted_argument_does_not_cast_siblings_or_nested_calls() {
        for call in [
            "accept({} --[[@as Message]], 'wrong')",
            "accept({text = integer_only('wrong')} --[[@as Message]], {} --[[@as Message]])",
            "integer_only({text = integer_only('wrong')} --[[@as integer]])",
        ] {
            let source = format!(
                r#"
            ---@class Message
            ---@field text string
            ---@param first Message
            ---@param second Message
            local function accept(first, second) end
            ---@param n integer
            local function integer_only(n) return n end
            {call}
            "#
            );
            let result = diagnostics(&source);
            assert_eq!(result.len(), 1, "{call}: {result:#?}");
            assert_eq!(result[0].code, DiagnosticCode::ParamTypeMismatch);
        }
    }

    #[test]
    fn test_incompatible_asserted_contract_is_still_rejected() {
        let source = r#"
        ---@class Message
        ---@field text string
        ---@param count integer
        local function accept(count) end
        accept({text = 'message'} --[[@as Message]])
        "#;
        let result = diagnostics(source);
        assert_eq!(result.len(), 1, "{result:#?}");
        assert_eq!(result[0].code, DiagnosticCode::ParamTypeMismatch);
    }

    #[test]
    fn test_asserted_arguments_in_methods_generics_and_variadics() {
        let source = r#"
        ---@class Message
        ---@field text string
        local methods = {}
        ---@param message Message
        function methods:accept(message) end
        ---@param ... Message
        local function accept_many(...) end
        ---@generic T
        ---@param value T
        ---@return T
        local function identity(value) return value end
        methods:accept({} --[[@as Message]])
        accept_many({} --[[@as Message]], {} --[[@as Message]])
        methods:accept(identity({} --[[@as Message]]))
        "#;
        assert!(diagnostics(source).is_empty(), "{:#?}", diagnostics(source));
    }

    #[test]
    fn test_unasserted_and_incompatible_contracts_still_report_errors() {
        for argument in ["{}", "{text = false}", "{} --[[@as integer]]"] {
            let source = format!(
                r#"
            ---@class Message
            ---@field text string
            ---@param message Message
            local function accept(message) end
            accept({argument})
            "#
            );
            assert!(!diagnostics(&source).is_empty(), "{argument}");
        }
    }

    #[test]
    fn test_asserted_argument_cache_preserves_nullable_contract() {
        use emmylua_parser::{LuaAstNode, LuaCallExpr};
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(
            r#"
            ---@class Message
            ---@field text string
            ---@param message Message?
            local function accept(message) end
            accept({} --[[@as Message?]])
        "#,
        );
        let model = ws.analysis.semantic_model(file);
        let call = model
            .chunk()
            .unwrap()
            .descendants::<LuaCallExpr>()
            .next()
            .unwrap();
        let analysis = model.call_site_analysis(&call);
        assert_eq!(analysis.arg_types, vec![ws.ty("Message?")]);
    }
}

#[cfg(test)]
mod test {
    use std::{ops::Deref, sync::Arc};

    use lsp_types::{Diagnostic, NumberOrString};
    use tokio_util::sync::CancellationToken;

    use crate::check::test::check_source;
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_function_argument_accepts_any_parameter() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file_id = ws.def(
            r#"
            ---@param value any
            local function accept(value) print(value) end
            ---@param f function
            local function use(f) accept(f) end
            use(function() end)
            "#,
        );
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn test_function_argument_preserves_narrow_parameter_checks() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file_id = ws.def(
            r#"
            ---@param value number
            local function accept_number(value) print(value) end
            ---@param callback fun(value: string): string
            local function accept_callback(callback) print(callback('hello')) end
            ---@param f function
            ---@param callback fun(value: number): number
            local function use(f, callback)
                accept_number(f)
                accept_callback(callback)
            end
            use(function() end, function(value) return value end)
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
            vec![Some(NumberOrString::String("param-type-mismatch".into())); 2],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn test_tuple_argument_accepts_valid_elements() {
        for (target, value) in [
            ("[integer, integer]", "{1, 2}"),
            ("[integer, integer]?", "{1, 2}"),
            ("Pair", "{1, 2}"),
            ("Pair?", "{1, 2}"),
            ("Pair?", "nil"),
            ("[integer, integer?]", "{1}"),
            ("[integer, integer?]", "{1, 2}"),
            ("[integer, integer|string]", "{1, \"text\"}"),
            ("Pair", "{1, 2, 3}"),
        ] {
            let diagnostics = check_source(&format!(
                r#"
                ---@alias Pair [integer, integer]
                ---@param pair {target}
                local function use_pair(pair) return pair end
                use_pair({value})
                "#,
            ));
            assert!(diagnostics.is_empty(), "{target}, {value}: {diagnostics:?}");
        }
    }

    #[test]
    fn test_tuple_argument_rejects_invalid_elements() {
        for (target, value) in [
            ("[integer, integer]", "{1, \"bad\"}"),
            ("[integer, integer]?", "{1, \"bad\"}"),
            ("Pair", "{1, \"bad\"}"),
            ("Pair?", "{1, \"bad\"}"),
            ("[integer, integer?]", "{1, \"bad\"}"),
            ("[integer, integer|boolean]", "{1, \"bad\"}"),
            ("Pair", "{1}"),
            ("Pair | [string, string] | boolean", "{1, \"bad\"}"),
        ] {
            let diagnostics = check_source(&format!(
                r#"
                ---@alias Pair [integer, integer]
                ---@param pair {target}
                local function use_pair(pair) return pair end
                use_pair({value})
                "#,
            ));
            assert_eq!(
                diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
                vec![DiagnosticCode::ParamTypeMismatch],
                "{target}, {value}: {diagnostics:?}"
            );
        }
    }

    fn param_type_diagnostics(ws: &mut VirtualWorkspace, block_str: &str) -> Vec<Diagnostic> {
        ws.analysis
            .diagnostic
            .enable_only(DiagnosticCode::ParamTypeMismatch);
        let file_id = ws.def(block_str);
        let code = Some(NumberOrString::String(
            DiagnosticCode::ParamTypeMismatch.get_name().to_string(),
        ));
        ws.analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap_or_default()
            .into_iter()
            .filter(|diagnostic| diagnostic.code == code)
            .collect()
    }

    #[test]
    fn test_any_unknown_accept_any_target() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param x number
            local function f(x) end

            ---@type any
            local a
            f(a)

            ---@type unknown
            local b
            f(b)
            "#
        ));
    }

    #[test]
    fn test_lambda_callback_no_mismatch() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@param callback fun(msg: string): boolean
                local function f(callback)
                end

                f(function(msg)
                    return true
                end)
            "#
        ));
    }

    #[test]
    fn test_param_type_mismatch_still_runs_when_count_diagnostics_disabled() {
        let mut ws = VirtualWorkspace::new();
        let diagnostics = param_type_diagnostics(
            &mut ws,
            r#"
                ---@param a string
                ---@param b string
                local function test(a, b)
                end

                test(1)
        "#,
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("string"),
            "{}",
            diagnostics[0].message
        );
    }

    #[test]
    fn test_issue_216() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@alias F1 fun(x: integer):integer
            do
                ---@type F1
                local test = function(x) return x + 1 end

                test("wrong type")
            end
        "#
        ));
    }

    #[test]
    fn test_issue_82() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@generic F: function
            ---@param _a F|integer
            ---@param _b? F
            ---@return F
            function foo(_a, _b)
                return _a
            end
            foo(function() end)
        "#
        ));
    }

    #[test]
    fn test_overload_param_type_mismatch_unions_failed_position() {
        let mut ws = VirtualWorkspace::new();
        let diagnostics = param_type_diagnostics(
            &mut ws,
            r#"
            ---@type fun(name: "游戏-初始化") | fun(name: "游戏-开始")
            local event
            local bad ---@type boolean

            event(bad)
        "#,
        );

        assert_eq!(diagnostics.len(), 1);
        let message = &diagnostics[0].message;
        assert!(message.contains("boolean"), "{message}");
        assert!(message.contains("游戏-初始化"), "{message}");
        assert!(message.contains("游戏-开始"), "{message}");
    }

    #[test]
    fn test_issue_75() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local a, b = pcall(string.rep, "a", "w")
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local a, b = pcall(string.rep, "a", 10000)
        "#
        ));
    }

    #[test]
    fn test_issue_85() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        ---@param a table | nil
        local function foo(a)
            a = a or {}
            _ = a.b
        end

        ---@param a table?
        local function _bar(a)
            a = a or {}
            _ = a.b
        end
        "#
        ));
    }

    #[test]
    fn test_issue_84() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        ---@param _a string[]
        local function bar(_a) end

        ---@param a? string[]?
        local function _foo(a)
            if not a then
                a = {}
            end

            bar(a)

            if not a then
                a = {}
            end

            bar(a)
        end
        "#
        ));
    }

    #[test]
    fn test_issue_83() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        ---@param _t table<any, string>
        local function foo(_t) end

        foo({})
        foo({'a'})
        foo({'a', 'b'})

        local a ---@type string[]
        foo(a)
        "#
        ));
    }

    #[test]
    fn test_issue_113() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@enum Baz
            local asd = {
                Foo = 0,
                Bar = 1,
                Baz = 2,
            }

            ---@param bob {a: Baz}
            function Foo(bob)
                return Bar(bob)
            end

            ---@param bob {a: Baz}
            function Bar(bob)
            end
        "#
        ));
    }

    #[test]
    fn test_issue_111() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        local Table = {}

        ---@param target table
        ---@param ... table
        ---@return table
        function Table.mergeInto(target, ...)
            -- Stuff
        end

        ---@param ... table
        ---@return table
        function Table.merge(...)
            return Table.mergeInto({}, ...)
        end
        "#
        ));
    }

    #[test]
    fn test_var_param_check() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
        ---@param target table
        ---@param ... table
        ---@return table
        function mergeInto(target, ...)
            -- Stuff
        end
        "#,
        );

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        mergeInto({}, 1)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        mergeInto({}, {}, {})
        "#
        ));
    }

    #[test]
    fn test_issue_102() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        ---@param _kind '' | 'Nr' | 'Ln' | 'Cul'
        function foo(_kind) end

        for _, kind in ipairs({ '', 'Nr', 'Ln', 'Cul' }) do
            foo(kind)
        end
        "#
        ));
    }

    #[test]
    fn test_issue_95() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        local range ---@type { [1]: integer, [2]: integer }

        table.sort(range)
        "#
        ));
    }

    #[test]
    fn test_issue_135() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        ---@alias A
        ---| "number" # A number

        ---@param a A
        local function f(a)
        end

        f("number")
        "#
        ));
    }

    #[test]
    fn test_colon_call_and_not_colon_define() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class Test
            local Test = {}

            ---@param a string
            function Test.name(a)
            end

            Test:name()
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class Test
            local Test = {}

            ---@param ... any
            function Test.dots(...)
            end

            Test:dots("a", "b", "c")
        "#
        ));
    }

    #[test]
    fn test_issue_148() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local a = (''):format()
        "#
        ));
    }

    #[test]
    fn test_generic_dots_param() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local d = select(1, 1, 2, 3)
        "#
        ));
    }

    #[test]
    fn test_bool_as_type() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        --- @param _x string|true
        function foo(_x) end

        foo(true)
        "#
        ));
    }

    #[test]
    fn test_function() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param sorter function
            ---@return string[]
            local function getTableKeys(sorter)
                local keys = {}
                table.sort(keys, sorter)
                return keys
            end
        "#
        ));
    }

    #[test]
    fn test_table_array() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@generic K, V
                ---@param t table<K, V>
                ---@return table<V, K>
                local function revertMap(t)
                end

                ---@param arr any[]
                local function sortCallbackOfIndex(arr)
                    ---@type table<any, integer>
                    local indexMap = revertMap(arr)
                end
        "#
        ));
    }

    #[test]
    fn test_table_class() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@param t table
                local function bar(t)
                end

                ---@class D11.A

                ---@type D11.A|any
                local a

                bar(a)
        "#
        ));
    }

    #[test]
    fn test_table_1() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@param t table[]
                local function bar(t)
                end

                ---@type table|any
                local a

                bar(a)
        "#
        ));
    }

    #[test]
    fn test_pairs() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@diagnostic disable: missing-return
                ---@generic K, V
                ---@param t table<K, V> | V[] | {[K]: V}
                ---@return fun(tbl: any):K, std.NotNull<V>
                local function _pairs(t) end

                ---@class D10.A

                ---@type {[string]: D10.A, _id: D10.A}
                local a

                for k, v in _pairs(a) do
                end
        "#
        ));
    }

    #[test]
    fn test_issue_278() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        local a --- @type type|'callable'
        error(a)  -- expected `string` but found `(type|"callable")`
        "#
        ))
    }

    #[test]
    fn test_issue_696() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        local ty --- @type type|fun(x:any): boolean

        --- @param _ty fun(v:any):boolean
        local function validate(_ty) end

        if type(ty) == 'function' then
          validate(ty)
        end
        "#
        ));
    }

    #[test]
    fn test_4() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class D13.Meta
            ---@field __defineGet fun(self: self, key: string, f: fun(self: self): any)

            ---@class D13.Impl: D13.Meta
            local impl = {}

            impl:__defineGet("value", function(self)
                return 1
            end)

            "#
        ));
    }

    #[test]
    fn test_issue_286() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                local a --- @type boolean
                local b --- @type integer?
                local c = a and b or nil
                -- type of c is (nil|true), should be (integer|nil)
                local d = a and b
                -- type of d is (boolean|nil), should be (false|integer|nil)

                ---@param p integer?
                local function f1(p)
                end
                f1(c)

                ---@param p false|integer|nil
                local function f2(p)
                end
                f2(d)
        "#
        ));
    }

    #[test]
    fn test_issue_287() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@param a table
                local function f(a)
                end

                ---@type table?
                local a
                a = a or {}

                f(a)
        "#
        ));
    }

    #[test]
    fn test_issue_336() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            --- @param b string
            --- @param c? boolean
            --- @param d? string
            --- @overload fun(b: string, d: string)
            function foo(b, c, d) end

            foo('number', true)
        "#
        ));
    }

    #[test]
    fn test_issue_348() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                local a --- @type type|'a'
                string.len(a)
        "#
        ));
    }

    #[test]
    fn test_super() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class py.ETypeMeta

                ---@class py.Vector3: py.ETypeMeta
                ---@class py.FVector3: py.ETypeMeta

                ---@alias Point.HandleType py.FVector3

                ---@class py.Point: py.Vector3

                ---@param point py.Point
                local function test(point)
                end

                ---@type Point.HandleType
                local handle

                test(handle)
        "#
        ));
    }

    #[test]
    fn test_union_type() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class py.Area
            ---@class py.RecArea: py.Area
            ---@class py.CirArea: py.Area

            ---@param a py.Area
            local function test(a)
            end

            ---@type py.RecArea | py.CirArea
            local a

            test(a)
        "#
        ));
    }

    #[test]
    fn test_super_1() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class py.SlotType: integer

                ---@param a py.SlotType
                local function test(a)
                end

                ---@type 0|1
                local a

                test(a)
        "#
        ));
    }

    #[test]
    fn test_alias_union_enum() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@alias EventType
                ---| GlobalEventType
                ---| UIEventType

                ---@enum UIEventType
                local UIEventType = {
                    ['UI_CREATE'] = "ET_UI_PREFAB_CREATE_EVENT",
                    ['UI_DELETE'] = "ET_UI_PREFAB_DEL_EVENT",
                }

                ---@enum GlobalEventType
                local GlobalEventType = {
                    ['GAME_INIT'] = "ET_GAME_INIT",
                    ['GAME_PAUSE'] = "ET_GAME_PAUSE",
                }

                ---@param event_name string
                local function get_py_event_name(event_name)
                end

                ---@param a EventType
                local function test(a)
                    get_py_event_name(a)
                end

        "#
        ));
    }

    #[test]
    fn test_alias_union_enum_2() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@alias EventType
                ---| GlobalEventType
                ---| UIEventType

                ---@enum UIEventType
                local UIEventType = {
                    ['UI_CREATE'] = "ET_UI_PREFAB_CREATE_EVENT",
                    ['UI_DELETE'] = "ET_UI_PREFAB_DEL_EVENT",
                }

                ---@enum GlobalEventType
                local GlobalEventType = {
                    ['GAME_INIT'] = 1,
                    ['GAME_PAUSE'] = "ET_GAME_PAUSE",
                }

                ---@param event_name string
                local function get_py_event_name(event_name)
                end

                ---@param a EventType
                local function test(a)
                    get_py_event_name(a)
                end

        "#
        ));
    }

    #[test]
    fn test_empty_class() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class D4.A: table<integer, string>

                ---@param lua_conf D4.A
                local function enable_global_lua_trigger(lua_conf) end

                ---@return { on_event: fun(trigger: table,  actor, data), [integer]: string }
                function new_global_trigger() end

                local a = new_global_trigger()

                enable_global_lua_trigger(a)
        "#
        ));
    }

    #[test]
    fn test_super_and_enum_1() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@enum AbilityType
                local AbilityType = {
                    HIDE   = 0,
                    NORMAL = 1,
                    COMMON = 2,
                    HERO   = 3,
                }
                ---@class py.AbilityType: integer

                ---@param ability_type py.AbilityType
                local function a(ability_type) end

                ---@param type AbilityType
                local function get(type)
                    local py_list = a(type)
                end
        "#
        ));
    }

    #[test]
    fn test_super_and_enum_2() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@enum AbilityType
                local AbilityType = {
                    HIDE   = "a",
                    NORMAL = 1,
                    COMMON = 2,
                    HERO   = 3,
                }
                ---@class py.AbilityType: integer

                ---@param ability_type py.AbilityType
                local function a(ability_type) end

                ---@param type AbilityType
                local function get(type)
                    local py_list = a(type)
                end
        "#
        ));
    }

    #[test]
    fn test_generic_array() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class LocalTimer

            ---@generic V
            ---@param list V[]
            ---@return integer
            local function sort(list) end

            ---@type { need_sort: true?, [integer]: LocalTimer }
            local queue = {}
            sort(queue)
        "#
        ));
    }

    #[test]
    fn test_function_union() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class (partial) D21.A
                local M

                ---@alias EventType
                ---| GlobalEventType
                ---| UIEventType

                ---@enum UIEventType
                local UIEventType = {
                    ['UI_CREATE'] = "ET_UI_PREFAB_CREATE_EVENT",
                }
                ---@enum GlobalEventType
                local GlobalEventType = {
                    ['GAME_INIT'] = "ET_GAME_INIT",
                }

                ---@param event_type EventType
                function M:event(event_type)
                end

                ---@class (partial) D21.A
                ---@field event fun(self: self, event: "游戏-初始化")
                ---@field event fun(self: self, event: "游戏-开始")

                ---@param p boolean
                local function test(p)
                    M:event(p)
                end
        "#
        ));
    }

    #[test]
    fn test_function_union_2() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class (partial) D21.A
                local M

                ---@alias EventType
                ---| GlobalEventType
                ---| UIEventType

                ---@enum UIEventType
                local UIEventType = {
                    ['UI_CREATE'] = "ET_UI_PREFAB_CREATE_EVENT",
                }
                ---@enum GlobalEventType
                local GlobalEventType = {
                    ['GAME_INIT'] = "ET_GAME_INIT",
                }

                ---@param event_type EventType
                function M:event(event_type)
                end

                ---@class (partial) D21.A
                ---@field event fun(self: self, event: "游戏-初始化")

                ---@param p EventType
                local function test(p)
                    M:event(p)
                end
        "#
        ));
    }

    #[test]
    fn test_function_union_meta() {
        let mut ws = VirtualWorkspace::new();
        let mut emmyrc = ws.analysis.emmyrc.as_ref().clone();
        emmyrc.strict.meta_override_file_define = false;
        ws.analysis.update_config(Arc::new(emmyrc));

        ws.def(
            r#"
                ---@meta
                ---@class (partial) D21.A
                ---@field event fun(self: self, event: "游戏-初始化")
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class (partial) D21.A
                local M

                ---@alias EventType
                ---| GlobalEventType
                ---| UIEventType

                ---@enum UIEventType
                local UIEventType = {
                    ['UI_CREATE'] = "ET_UI_PREFAB_CREATE_EVENT",
                }
                ---@enum GlobalEventType
                local GlobalEventType = {
                    ['GAME_INIT'] = "ET_GAME_INIT",
                }

                ---@param event_type EventType
                function M:event(event_type)
                end

                ---@param p EventType
                local function test(p)
                    M:event(p)
                end
        "#
        ));
    }

    #[test]
    fn test_function_self() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class A

                ---@param init fun(self: self)
                local function test(init)
                end

                ---@param superInit fun(self: A)
                local function declare(superInit)
                    test(superInit)
                end
        "#
        ));
    }

    #[test]
    fn test_self_1() {
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
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local A = require("1")

            ---@class D31.B
            local B = {}

            function B:__init()
                self.originalExecute = A.execute
                A.execute = function(trg, ...)
                    self.originalExecute(trg, ...)
                end
            end
        "#
        ));
    }

    #[test]
    fn test_flow_alias() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let mut emmyrc = ws.analysis.get_emmyrc().deref().clone();
        emmyrc.strict.array_index = false;
        ws.analysis.update_config(Arc::new(emmyrc));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class Trigger
                ---@alias Trigger.CallBack fun(trg: Trigger, ...): any, any, any, any

                ---@param callback Trigger.CallBack
                local function event(callback)
                end

                local function core_subscribe(...)
                    ---@type Trigger.CallBack
                    local callback
                    local nargs = select('#', ...)
                    if nargs == 1 then
                        callback = ...
                    elseif nargs > 1 then
                        extra_args = { ... }
                        callback = extra_args[nargs]
                    end

                    local b = event(callback)
                end
        "#
        ));
    }

    #[test]
    fn test_issue_487() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param start string
            ---@return boolean
            function string:startswith(start)
                return self:sub(1, #start) == start
            end
        "#
        ));
    }

    #[test]
    fn test_int() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param count integer
            local function loop_count(count)
            end

            loop_count(45 / 3)
        "#
        ));
    }

    #[test]
    fn test_int_to_alias() {
        let mut ws = VirtualWorkspace::new();
        let mut emmyrc = ws.analysis.get_emmyrc().deref().clone();
        emmyrc.strict.doc_base_const_match_base_type = true;
        ws.analysis.update_config(Arc::new(emmyrc));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@alias IdAlias
                ---| 311000001
                ---| 311000002

                ---@param id IdAlias
                local function f(id)
                end

                ---@type integer
                local a
                f(a)
        "#
        ));
    }

    #[test]
    fn test_enum_value_matching() {
        let mut ws = VirtualWorkspace::new();
        let mut emmyrc = ws.analysis.get_emmyrc().deref().clone();
        emmyrc.strict.doc_base_const_match_base_type = true;
        ws.analysis.update_config(Arc::new(emmyrc));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@enum SlotType
                local SlotType = {
                    bag = 0,
                    item = 1,
                }

                ---@enum ConstSlotType
                local ConstSlotType = {
                    ['NOT_IN_BAG'] = -1,
                    ['PKG'] = 0,
                    ['BAR'] = 1,
                }

                ---@param type ConstSlotType
                local function get_item_by_slot(type)
                end

                ---@param field SlotType
                local function bind_unit_slot(field)
                    get_item_by_slot(field)
                end"#
        ));
    }

    #[test]
    fn test_enum_value_matching_2() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@enum DamageType
                local DamageType = {
                    ['物理'] = "物理",
                }
                for _, damageType in pairs(DamageType) do end
            "#
        ));
    }

    #[test]
    fn test_super_type_match() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class UnitKey: integer

                ---@alias IdAlias
                ---| 10101

                ---@param key IdAlias
                local function get(key)
                end

                ---@type UnitKey
                local key

                get(key)
            "#
        ));
    }

    #[test]
    fn test_self() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class test
                local A

                function A:stop()
                end

                local stop = A.stop
                stop(A)
            "#
        ));
    }

    #[test]
    fn test_generic_type() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
                ---@class ObserverParams<T>
                ---@field next fun(value: T)
                ---@field errorResume? fun(error: any)


                ---@class Observer<T>
                local Observer = {}

                ---@param observer ObserverParams<T>
                function Observer:subscribe(observer)
                end
        "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@type Observer<number>
            local observer

            observer:subscribe({
                next = function(value)
                    print(value)
                end
            })
            "#
        ));
    }

    #[test]
    fn test_issue_573() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
                --- @class A
                --- @field [integer] string
                --- @field data any

                --- @param a string[]
                function takesArray(a) end
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                takesArray({} --[[@as A]])
            "#
        ));
    }

    #[test]
    fn test_issue_574() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
                --- @param x { y: integer } & { z: string }
                function foo(x) end
            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
               foo({y = "", z = ""})
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
               foo({y = 1, z = ""})
            "#
        ));
    }

    #[test]
    fn test_meta_pairs() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
                ---@class RingBufferSpan<T>
                local RingBufferSpan

                ---@return fun(): integer, T
                function RingBufferSpan:__pairs()
                end
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local pairs = pairs

            ---@type RingBufferSpan
            local span

            for k, v in pairs(span) do
            end
            "#
        ));

        // Test generics.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@type RingBufferSpan<number>
            local span

            for k, v in pairs(span) do
                A = v
            end
            "#
        ));
        let a_ty = ws.expr_ty("A");
        let expected = ws.ty("number");
        assert_eq!(a_ty, expected);
    }

    #[test]
    fn test_generic_union_type() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
                ---@class Params<T>
                ---@field next fun(value: T)
                ---@field error? fun(error: any)

                ---@generic T
                ---@param params fun(value: T) | Params<T>
                function test(params)
                end

            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                test({})
            "#
        ));
    }

    #[test]
    fn test_alias_branch_label_flow() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "test.lua",
            r#"
            ---@alias EditorAttrTypeAlias
            ---| 'ATTR_BASE'
            ---| 'ATTR_BASE_RATIO'
            ---| 'ATTR_ALL_RATIO'

            ---@param attr_element string
            function test(attr_element) end
        "#,
        );

        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param attr_type EditorAttrTypeAlias
            function add_attr(attr_type)
                if attr_type ~= 'ATTR_BASE' then
                end
                test(attr_type)
            end
        "#
        ));
    }

    #[test]
    fn test_self_contain_tpl() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "test.lua",
            r#"
                ---@class Observable<T>
                Observable = {}

                ---@param ... Observable<any>
                function zip(...)
                end

        "#,
        );

        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            function Observable:test()
                zip(self)
            end
        "#
        ));
    }

    #[test]
    fn test_issue_841() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        --- @class B
        --- @field cmd string

        --- @class A: B
        --- @field cmd? string

        --- @param x A
        local function foo(x)
        end

        foo({})
        "#,
        ));
    }

    #[test]
    fn test_fix_issue_844() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
        ---@alias Tester fun(customTesters: Tester[]): boolean?

        ---@generic V
        ---@param t V[]
        ---@return fun(tbl: any):int, V
        function ipairs(t) end
        "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param newTesters Tester[]
            local function addMatchers(newTesters)
                for _, tester in ipairs(newTesters) do
                end
            end
        "#
        ));
    }

    #[test]
    fn test_pairs_1() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
        ---@param value string
        function aaaa(value)
        end

        ---@generic K, V
        ---@param t {[K]: V} | V[]
        ---@return fun(tbl: any):K, V
        function pairs(t) end
        "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@type {[string]: number}
            local matchers = {}
            for _, matcher in pairs(matchers) do
                aaaa(matcher)
            end
        "#
        ));
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@alias MatchersObject {[string]: number}
            ---@type MatchersObject
            local matchers = {}
            for _, matcher in pairs(matchers) do
                aaaa(matcher)
            end
        "#
        ));
    }

    #[test]
    fn test_keyof() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class SuiteHooks
            ---@field beforeAll string
            ---@field afterAll string

            ---@param name keyof SuiteHooks
            function test(name)
            end
        "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            test("a")
        "#,
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            test("beforeAll")
        "#,
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@type keyof SuiteHooks
            local name
            test(name)
        "#,
        ));
    }

    #[test]
    fn test_index_access_with_keyof_alias() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class A
            ---@field one 1

            ---@alias KeyofA keyof A
            ---@type A[KeyofA]
            local tmp

            ---@param v 1
            local function test(v)
            end

            test(tmp)
        "#,
        ));
    }

    #[test]
    fn test_origin_self() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class Runner
            ---@field onCollectStart? fun(self:self)

            ---@type Runner
            runner = {}

        "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local originOnCollectStart = runner.onCollectStart
            runner.onCollectStart = function(self)
                originOnCollectStart(self)
            end
        "#,
        ));
    }

    #[test]
    fn test_issue_896() {
        let mut ws = VirtualWorkspace::new();
        let mut emmyrc = ws.get_emmyrc();
        emmyrc.strict.array_index = false;
        ws.update_emmyrc(emmyrc);

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@alias MyFnA fun(): number
            ---@alias MyFnB<T> fun(): T
            ---@alias MyFnC<T> fun(): number

            local aaa ---@type MyFnA[]
            ---@param aaa MyFnA[]
            local function AAA(aaa) return aaa end
            local _ = AAA(aaa)

            local bbb1 ---@type (MyFnB<number>)[]
            local bbb2 = { function() return 1 end } ---@type (MyFnB<number>)[]
            ---@param bbb (MyFnB<number>)[]
            local function BBB(bbb) return bbb end
            local _ = BBB(bbb1)
            local _ = BBB(bbb2)

            local ccc1 ---@type (MyFnC<number>)[]
            local ccc2 = { function() return 1 end } ---@type (MyFnC<number>)[]
            ---@param ccc (MyFnC<number>)[]
            local function CCC(ccc) return ccc end
            local _ = CCC(ccc1)
            local _ = CCC(ccc2)
        "#,
        ));
    }

    #[test]
    fn test_call_operator_implicit_self_param_type() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class Iter

                ---@class IterMod
                ---@operator call: Iter

                ---@type string[]
                local paths = { "a" }

                ---@type IterMod
                local iter

                local value = iter(paths)
        "#,
        ));
    }

    #[test]
    fn test_colon_call_param_check_uses_receiver_when_member_is_callable() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class Owner
                ---@field run CallableMethod

                ---@class CallableMethod
                ---@overload fun(owner: Owner)

                ---@type Owner
                local owner

                owner:run()
        "#,
        ));
    }

    #[test]
    fn test_generic_infer_function_2() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@generic T: table
            ---@param table T
            ---@param metatable table|nil
            ---@return T
            function setmetatable(table, metatable) end

            ---@alias PartialFunction<T> { [P in keyof T]: T[P] extends function and T[P]? or T[P]; }

            ---@class MockContext<T>

            ---@class Mock<T>
            ---@field mock MockContext<T>
            Mock = {}
            "#,
        );

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@type PartialFunction<Mock>
            local mock

            setmetatable(mock, Mock)
        "#,
        ));
    }

    #[test]
    fn test_generic_variability() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
                ---@generic T
                ---@param ... T...
                function test(...) end
            "#,
        );

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@type string?
                local message
                test(message)
                test(message, message)
        "#,
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local filename = 'flag.text'
            local d = io.open(filename, 'r')
            assert(d)
        "#,
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            local filename = 'flag.text'
            assert(io.open(filename, 'r'))
            "#,
        ));
    }

    #[test]
    fn test_generic_constraint_arg_to_incompatible_param() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class Animal
            ---@field name string

            ---@param value string
            local function takeString(value)
            end

            ---@generic T: Animal
            ---@param animal T
            local function checkAnimal(animal)
                takeString(animal)
            end
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@class Animal
            ---@field name string

            ---@param value Animal
            local function takeAnimal(value)
            end

            ---@generic T: Animal
            ---@param animal T
            local function checkAnimal(animal)
                takeAnimal(animal)
            end
        "#
        ));
    }

    #[test]
    fn test_generic_callback_parameter_contravariance() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class A
            ---@class B
            ---@class C
            ---@class D

            ---@param a A | B | C
            ---@return boolean
            function condition(a)
                return true
            end

            ---@generic T
            ---@param a T[]
            ---@param b fun(a: T): boolean
            function test(a, b) end
            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@type (A | B | C | D)[]
            local ABCD
            test(ABCD, condition)
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@type (A | B)[]
            local AB
            test(AB, condition)
            "#
        ));
    }

    #[test]
    fn test_unknown_alias_param_accepts() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@alias Anything unknown
                ---@param s string
                local function f(s) end

                ---@type Anything
                local a
                f(a)
            "#
        ));
    }

    #[test]
    fn test_dot_call_colon_defined_method_uses_correct_arg_type() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class Foo
                ---@field value number
                local Foo = {}

                ---@param x number
                function Foo:method(x) end

                ---@type Foo
                local foo
                Foo.method(foo, 1)
            "#
        ));
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@class Foo
                ---@field value number
                local Foo = {}

                ---@param x Foo
                function Foo:method(x) end

                ---@type Foo
                local foo
                Foo.method(foo, 1)
            "#
        ));
    }

    /// Fewer arguments than the variadic slot position: the variadic parameter
    /// consumes an empty slice instead of panicking on `args[index..]`.
    #[test]
    fn test_variadic_slot_beyond_args_no_panic() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@param a string
                ---@param b string
                ---@param ... number
                local function f(a, b, ...) end
                f("x")
            "#
        ));
    }

    /// Same for a generic variadic slot (`...: T...`), which has its own early path.
    #[test]
    fn test_generic_variadic_slot_beyond_args_no_panic() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@generic T
                ---@param a string
                ---@param ... T...
                local function f(a, ...) end
                f("x")
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@generic T
                ---@param a string
                ---@param ... T...
                local function f(a, ...) end
                f()
            "#
        ));
    }

    /// No arguments at all, so the variadic slot index is past the end of `args`.
    #[test]
    fn test_variadic_slot_no_args_no_panic() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@param a string
                ---@param ... number
                local function f(a, ...) end
                f()
            "#
        ));
    }

    /// The variadic slot still reports mismatches for the arguments it does receive.
    #[test]
    fn test_variadic_slot_still_checks_provided_args() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
                ---@param a string
                ---@param b string
                ---@param ... number
                local function f(a, b, ...) end
                f("x", "y", "z")
            "#
        ));
    }

    /// Full default checker pipeline (the path behind `textDocument/diagnostic`) on the
    /// crashing input: no panic, and the missing named argument is still reported.
    #[test]
    fn test_variadic_slot_beyond_args_full_check_pipeline() {
        let diagnostics = check_source(
            r#"
                ---@param a string
                ---@param b string
                ---@param ... number
                local function f(a, b, ...) end
                f("x")
            "#,
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingParameter)
                .count(),
            1,
            "diagnostics: {:?}",
            diagnostics
        );
    }
}
#[cfg(test)]
mod nominal_enum_members {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use googletest::prelude::*;

    #[gtest]
    fn enum_accepts_its_nominal_members() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            defines = {}
            ---@enum defines.rail_direction
            defines.rail_direction = {
                ---@class defines.rail_direction.back
                back = {},
                ---@class defines.rail_direction.front
                front = {},
            }
            ---@param direction defines.rail_direction
            function use_direction(direction) end
        "#,
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::ParamTypeMismatch,
                r#"
            ---@type defines.rail_direction.front
            local front
            use_direction(front)
            use_direction(defines.rail_direction.front)
            use_direction(defines.rail_direction.back)
        "#
            ),
            eq(true)
        );
    }

    #[gtest]
    fn enum_accepts_predeclared_class_value() {
        let mut ws = VirtualWorkspace::new();
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::ParamTypeMismatch,
                r#"
            ---@class Front
            local front = {}
            ---@enum Direction
            local Direction = {front = front}
            ---@param direction Direction
            local function use_direction(direction) end
            use_direction(front)
        "#
            ),
            eq(true)
        );
    }

    #[gtest]
    fn enum_member_compatibility_preserves_rejections() {
        let definitions = r#"
            defines = {}
            ---@enum Direction
            defines.directions = {
                ---@class Front
                front = {},
            }
            ---@enum OtherDirection
            defines.other = {
                ---@class OtherFront
                front = {},
            }
            ---@class Direction.unlisted
            ---@class Unrelated
            ---@param direction Direction
            function use_direction(direction) end
        "#;
        for ty in [
            "OtherFront",
            "Direction.unlisted",
            "Unrelated",
            "nil",
            "Front?",
        ] {
            let mut ws = VirtualWorkspace::new();
            ws.def(definitions);
            assert_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::ParamTypeMismatch,
                    &format!(
                        r#"
                ---@param value {ty}
                local function call(value) use_direction(value) end
            "#
                    )
                ),
                eq(false),
                "{ty}"
            );
        }
    }

    #[gtest]
    fn enum_member_assignments_and_optional_parameters() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class Front
            front = {}
            ---@class Back
            back = {}
            ---@class Other
            other = {}
            ---@enum Direction
            directions = {front = front, back = back}
            ---@param direction Direction?
            function optional_direction(direction) end
        "#,
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::AssignTypeMismatch,
                r#"
            ---@type Direction
            local direction = front
            direction = back
        "#
            ),
            eq(true)
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::AssignTypeMismatch,
                r#"
            ---@type Direction
            local direction = other
        "#
            ),
            eq(false)
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::ParamTypeMismatch,
                r#"
            optional_direction(front)
            optional_direction(back)
            optional_direction(nil)
        "#
            ),
            eq(true)
        );
    }

    #[gtest]
    fn nested_enum_and_inherited_member() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            defines = {
                ---@enum Direction
                direction = {
                    ---@class Front
                    front = {},
                },
            }
            ---@class DerivedFront: Front
            derived_front = {}
            ---@param direction Direction
            function use_direction(direction) end
        "#,
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::ParamTypeMismatch,
                r#"
            use_direction(derived_front)
        "#
            ),
            eq(true)
        );
    }

    #[gtest]
    fn recursive_enum_member_does_not_accept_unrelated_class() {
        let mut ws = VirtualWorkspace::new();
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::ParamTypeMismatch,
                r#"
            ---@class Other
            local other = {}
            ---@type Recursive
            local recursive
            ---@enum Recursive
            local values = {self = recursive}
            ---@param value Recursive
            local function use(value) end
            use(other)
        "#
            ),
            eq(false)
        );
    }

    #[gtest]
    fn private_enum_member_does_not_match_another_files_class() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class (private) Front
            local front = {}
            ---@enum Direction
            local values = {front = front}
            ---@param value Direction
            function use_direction(value) end
        "#,
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::ParamTypeMismatch,
                r#"
            ---@class (private) Front
            local front = {}
            use_direction(front)
        "#
            ),
            eq(false)
        );
    }
}

#[cfg(test)]
mod callback_return_aliases {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use googletest::prelude::*;

    fn workspace() -> VirtualWorkspace {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            ---@alias Message [string, Message...]|number|boolean|string
            ---@alias Callback fun():Message
            ---@param callback Callback
            function print_message(callback) end
        "#,
        );
        ws
    }

    #[gtest]
    fn accepts_alias_return_alternatives() {
        for body in [
            "return 'hello'",
            "return 1",
            "return true",
            "return {'hello'}",
            "return {'hello', 1, false, 'world', {'nested', 2}}",
            "return {'hello'} --[[@as Message]]",
            "local message = {'hello'}; return message",
            "local message = {'hello', 1}; return message",
        ] {
            let mut ws = workspace();
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::ParamTypeMismatch,
                    &format!("print_message(function() {body}\nend)")
                ),
                eq(true),
                "{body}"
            );
        }
    }

    #[gtest]
    fn rejects_invalid_alias_returns() {
        for body in [
            "return nil",
            "return function() end",
            "return {}",
            "return {42}",
            "return {'hello', function() end}",
            "return {'hello', 1, 2, function() end}",
            "return {'hello', {'nested', function() end}}",
        ] {
            let mut ws = workspace();
            expect_that!(
                ws.has_no_diagnostic(
                    DiagnosticCode::ParamTypeMismatch,
                    &format!("print_message(function() {body}\nend)")
                ),
                eq(false),
                "{body}"
            );
        }
    }

    #[gtest]
    fn aliases_and_inline_unions_agree() {
        for annotation in ["Scalar", "ScalarChain", "string|number"] {
            for (body, valid) in [
                ("return 'text'", true),
                ("return 1", true),
                ("return false", false),
            ] {
                let mut ws = VirtualWorkspace::new_with_init_std_lib();
                ws.def(
                    r#"
                    ---@alias Scalar string|number
                    ---@alias ScalarChain Scalar
                "#,
                );
                let code = format!(
                    "---@param callback fun():{annotation}\nlocal function use(callback) end\nuse(function() {body} end)"
                );
                expect_that!(
                    ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, &code),
                    eq(valid),
                    "{annotation}: {body}"
                );
            }
        }
    }

    #[gtest]
    fn callback_parameter_aliases_preserve_contravariance() {
        for (actual, expected, valid) in [
            ("Scalar", "string", true),
            ("string", "Scalar", false),
            ("Scalar", "string|number", true),
            ("string|number", "Scalar", true),
            ("boolean", "Scalar", false),
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(r#"---@alias Scalar string|number"#);
            let crate::LuaType::DocFunction(actual_fn) = ws.ty(&format!("fun(value: {actual})"))
            else {
                panic!("function type");
            };
            let crate::LuaType::DocFunction(expected_fn) =
                ws.ty(&format!("fun(value: {expected})"))
            else {
                panic!("function type");
            };
            let file = ws.def("");
            let model = ws.analysis.semantic_model(file);
            let result = crate::semantic_model::infer::function_solver::functions_compatible(
                &model,
                &actual_fn,
                &expected_fn,
                &Default::default(),
            );
            expect_that!(result.compatible, eq(valid), "{actual} -> {expected}");
        }
    }

    #[gtest]
    fn callback_return_unions_and_cycles() {
        for (actual, expected, valid) in [
            ("Scalar", "string|number", true),
            ("string|number", "Scalar", true),
            ("Scalar?", "Scalar", false),
            ("Scalar", "Scalar?", true),
            ("string|boolean", "Scalar", false),
            ("Cycle", "string", false),
            ("string", "Cycle", false),
            ("string", "Recursive", true),
            ("boolean", "Recursive", false),
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(
                r#"
                ---@alias Scalar string|number
                ---@alias Cycle Cycle
                ---@alias Recursive Recursive|string
            "#,
            );
            let crate::LuaType::DocFunction(actual_fn) = ws.ty(&format!("fun():{actual}")) else {
                panic!("function type");
            };
            let crate::LuaType::DocFunction(expected_fn) = ws.ty(&format!("fun():{expected}"))
            else {
                panic!("function type");
            };
            let file = ws.def("");
            let model = ws.analysis.semantic_model(file);
            let result = crate::semantic_model::infer::function_solver::functions_compatible(
                &model,
                &actual_fn,
                &expected_fn,
                &Default::default(),
            );
            expect_that!(result.compatible, eq(valid), "{actual} -> {expected}");
        }
    }

    #[gtest]
    fn tuple_tails_check_every_present_element() {
        for (value, valid) in [
            ("{'head'}", true),
            ("{'head', 1, 2, 3}", true),
            ("{}", false),
            ("{1}", false),
            ("{'head', 'bad'}", false),
            ("{'head', 1, 2, 'bad'}", false),
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            let code = format!(
                r#"
                ---@param callback fun():[string, number...]
                local function use(callback) end
                use(function() return {value} end)
            "#
            );
            expect_that!(
                ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, &code),
                eq(valid),
                "{value}"
            );
        }
    }
}

#[test]
fn test_builtin_length_argument_type() {
    use crate::DiagnosticCode;
    use crate::check::test::check_source;
    let check_source = |source: &str| {
        check_source(source)
            .into_iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::ParamTypeMismatch)
            .collect::<Vec<_>>()
    };
    let prelude = r#"
        ---@class Train
        ---@class Metrics
        local Metrics = {}
        ---@param key string
        ---@param value integer
        function Metrics:set(key, value) end
        ---@type Train[]
        local selected_trains = {}
        ---@param value integer
        local function integer_value(value) end
        ---@param value Train[]
        local function train_values(value) end
        ---@param value string
        local function string_value(value) end
    "#;
    for expression in [
        "Metrics:set('match_count', #selected_trains)",
        "integer_value(#selected_trains)",
        "integer_value(#{})",
        "integer_value(#'train')",
    ] {
        let diagnostics = check_source(&format!("{prelude}\n{expression}"));
        assert!(diagnostics.is_empty(), "{expression}: {diagnostics:#?}");
    }
    for expression in [
        "Metrics:set('match_count', selected_trains)",
        "train_values(#selected_trains)",
        "string_value(#'train')",
    ] {
        let diagnostics = check_source(&format!("{prelude}\n{expression}"));
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![DiagnosticCode::ParamTypeMismatch],
            "{expression}: {diagnostics:#?}"
        );
    }
}

#[test]
fn test_length_argument_preserves_operator_overload() {
    use crate::DiagnosticCode;
    use crate::check::test::check_source;
    let check_source = |source: &str| {
        check_source(source)
            .into_iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::ParamTypeMismatch)
            .collect::<Vec<_>>()
    };
    let prelude = r#"
        ---@class Collection
        ---@operator len: string
        local Collection = {}
        ---@type Collection
        local values
        ---@param value string
        local function string_value(value) end
        ---@param value integer
        local function integer_value(value) end
    "#;
    assert!(check_source(&format!("{prelude}\nstring_value(#values)")).is_empty());
    let diagnostics = check_source(&format!("{prelude}\ninteger_value(#values)"));
    assert_eq!(
        diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
        vec![DiagnosticCode::ParamTypeMismatch],
        "{diagnostics:#?}"
    );
}

#[test]
fn test_length_argument_preserves_nullable_member_diagnostic() {
    use crate::DiagnosticCode;
    use crate::check::test::{check_source, count_by_code};
    let diagnostics = check_source(
        r#"
        ---@class Container
        ---@field values string[]
        ---@param value integer
        local function integer_value(value) return value end
        ---@param container Container?
        function length(container)
            integer_value(#container.values)
        end
    "#,
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::NeedCheckNil),
        1,
        "{diagnostics:#?}"
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
        0,
        "{diagnostics:#?}"
    );
}

#[cfg(test)]
mod class_index_array_compatibility {
    use crate::DiagnosticCode;
    use crate::check::test::check_source;

    fn mismatch_count(declaration: &str, source: &str, target: &str) -> usize {
        let code = format!(
            r#"
            {declaration}
            ---@param value {target}[]
            local function accept(value) return value end
            ---@type {source}
            local value
            accept(value)
        "#
        );
        check_source(&code)
            .into_iter()
            .filter(|d| d.code == DiagnosticCode::ParamTypeMismatch)
            .count()
    }

    #[test]
    fn named_numeric_index_signature_is_an_array() {
        assert_eq!(
            mismatch_count(
                r#"
            ---@class StringList
            ---@field [integer] string
            ---@field data any
            local list = {}
        "#,
                "StringList",
                "string"
            ),
            0
        );
    }

    #[test]
    fn numeric_signatures_support_inheritance_aliases_and_generics() {
        for (declaration, source, target) in [
            (
                r#"
                ---@class NumberKeys
                ---@field [number] string
                local list = {}
            "#,
                "NumberKeys",
                "string",
            ),
            (
                r#"
                ---@class BaseList
                ---@field [integer] string
                local base = {}
                ---@class DerivedList: BaseList
                ---@field metadata boolean
                local derived = {}
                ---@alias ListAlias DerivedList
            "#,
                "ListAlias",
                "string",
            ),
            (
                r#"
                ---@class GenericList<T>
                ---@field [integer] T
                local list = {}
            "#,
                "GenericList<string>",
                "string",
            ),
            (
                r#"
                ---@class OptionalList
                ---@field [integer] string?
                local list = {}
            "#,
                "OptionalList",
                "(string?)",
            ),
            (
                r#"
                ---@class IntegerList
                ---@field [integer] integer
                local list = {}
            "#,
                "IntegerList",
                "number",
            ),
        ] {
            assert_eq!(
                mismatch_count(declaration, source, target),
                0,
                "{source} -> {target}[]"
            );
        }
    }

    #[test]
    fn incompatible_or_absent_signatures_are_rejected() {
        for (field, operator) in [
            ("[integer] number", ""),
            ("[integer] string?", ""),
            ("[string] string", ""),
            ("[1] string", ""),
            ("[\"integer\"] string", ""),
            ("data string", "---@operator len: integer"),
        ] {
            let declaration = format!(
                r#"
                ---@class NotAStringArray
                ---@field {field}
                {operator}
                local list = {{}}
            "#
            );
            assert_eq!(
                mismatch_count(&declaration, "NotAStringArray", "string"),
                1,
                "{field}"
            );
        }
        let generic = r#"
            ---@class GenericList<T>
            ---@field [integer] T
            local list = {}
        "#;
        assert_eq!(mismatch_count(generic, "GenericList<number>", "string"), 1);
    }

    #[test]
    fn specific_and_derived_signatures_take_precedence() {
        let specific = r#"
            ---@class Indexed
            ---@field [integer] string
            ---@field [number] boolean
            local list = {}
        "#;
        assert_eq!(mismatch_count(specific, "Indexed", "string"), 0);
        assert_eq!(mismatch_count(specific, "Indexed", "boolean"), 1);
        let inherited = r#"
            ---@class ParentList
            ---@field [integer] string
            local parent = {}
            ---@class ChildList: ParentList
            ---@field [integer] number
            local child = {}
        "#;
        assert_eq!(mismatch_count(inherited, "ChildList", "number"), 0);
        assert_eq!(mismatch_count(inherited, "ChildList", "string"), 1);
    }
}
