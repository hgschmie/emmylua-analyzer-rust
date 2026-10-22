#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_mutable_capture_unrelated_closures_preserve_diagnostics() {
        for (code, source) in [
            (
                DiagnosticCode::UnnecessaryIf,
                r#"
                local value = 0
                local function unrelated() return 1 end
                if value == 0 then print(value) end
            "#,
            ),
            (
                DiagnosticCode::UnnecessaryIf,
                r#"
                local value = 0
                local function unrelated() local value = 1; value = 2 end
                local function read() if value == 0 then print(value) end end
            "#,
            ),
            (
                DiagnosticCode::UnnecessaryIf,
                r#"
                local value = 0
                value = 1
                local function unrelated() return 2 end
                local function read() if value == 1 then print(value) end end
            "#,
            ),
            (
                DiagnosticCode::NeedCheckNil,
                r#"
                ---@type {name: string}?
                local value
                local function unrelated() return 1 end
                local function read() return value.name end
            "#,
            ),
        ] {
            let mut ws = VirtualWorkspace::new();
            assert!(!ws.has_no_diagnostic(code, source), "{source}");
        }
    }

    #[test]
    fn test_mutable_capture_recursive_queries_keep_the_same_types() {
        use crate::LuaType;
        use emmylua_parser::{LuaAstNode, LuaAstToken, LuaLocalName};
        let source = r#"
            local first, second = 0, 0
            local function write_first() first = second + 1 end
            local function write_second() second = first + 1 end
            local function read()
                local observed_first = first
                local observed_second = second
                if first == 0 then print(first) end
                if second == 0 then print(second) end
            end
        "#;
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(source);
        for order in [
            ["observed_first", "observed_second"],
            ["observed_second", "observed_first"],
        ] {
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            for name in order.into_iter().cycle().take(4) {
                let local = chunk
                    .descendants::<LuaLocalName>()
                    .find(|node| {
                        node.get_name_token()
                            .is_some_and(|token| token.get_name_text() == name)
                    })
                    .unwrap();
                let decl = model
                    .decl_by_offset(local.get_name_token().unwrap().get_position())
                    .unwrap();
                assert_eq!(
                    model.type_of_decl(&decl),
                    Some(LuaType::from_vec(vec![
                        LuaType::IntegerConst(0),
                        LuaType::Integer
                    ])),
                    "{name}"
                );
            }
        }
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, source));
    }

    #[test]
    fn test_mutable_capture_annotations_keep_contract_and_check_writes() {
        for invalid in ["nil", "'wrong'"] {
            for (setup, finish) in [
                ("---@type boolean\nlocal state = false", ""),
                (
                    "---@param unused integer\n---@param state boolean\nlocal function factory(state, unused)",
                    "end",
                ),
                (
                    "---@param state boolean\nlocal function factory(unused, state)",
                    "end",
                ),
            ] {
                let source = format!(
                    r#"
                    {setup}
                    ---@return boolean
                    local function read() return state end
                    local function write() state = {invalid} end
                    {finish}
                "#
                );
                let mut ws = VirtualWorkspace::new();
                assert!(
                    !ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
                    "{source}"
                );
                let mut ws = VirtualWorkspace::new();
                assert!(
                    ws.has_no_diagnostic(DiagnosticCode::ReturnTypeMismatch, &source),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn test_mutable_capture_named_function_writes_contribute_values() {
        for writer in [
            "function action() return true end",
            "local function write() function action() return true end end",
        ] {
            let source = format!(
                r#"
                local action = nil
                local function read()
                    observed = action
                    if action == nil then print('not ready') end
                end
                {writer}
            "#
            );
            let mut ws = VirtualWorkspace::new();
            ws.def(&source);
            let observed = ws.expr_ty("observed");
            let crate::LuaType::Union(types) = observed else {
                panic!("expected both nil and the named function, got {observed:?}");
            };
            assert!(types.iter().any(crate::LuaType::is_nil));
            assert!(types.iter().any(crate::LuaType::is_function));
            assert!(types.iter().all(|ty| ty.is_nil() || ty.is_function()));
            let mut ws = VirtualWorkspace::new();
            assert!(
                ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, &source),
                "{source}"
            );
        }
    }

    #[test]
    fn test_mutable_capture_named_function_unfinished_writes_stay_nullable() {
        for source in [
            r#"
                ---@type (fun(): boolean)?
                local action
                ---@return fun(): boolean
                local function read() return action end
                function action() return true end
            "#,
            r#"
                ---@type (fun(): boolean)?
                local action
                local function write() function action() return true end end
                ---@return fun(): boolean
                local function read() return action end
            "#,
            r#"
                ---@type (fun(): boolean)?
                local action
                while unknown do function action() return true end end
                ---@return fun(): boolean
                local function read() return action end
            "#,
            r#"
                ---@type (fun(): boolean)?
                local action
                if unknown then function action() return true end end
                ---@return fun(): boolean
                local function read() return action end
            "#,
            r#"
                ---@type (fun(): boolean)?
                local action
                ::again::
                action = nil
                ---@return fun(): boolean
                local function read() return action end
                function action() return true end
                if unknown then goto again end
            "#,
        ] {
            let mut ws = VirtualWorkspace::new();
            assert!(
                !ws.has_no_diagnostic(DiagnosticCode::ReturnTypeMismatch, source),
                "{source}"
            );
        }
    }

    #[test]
    fn test_mutable_capture_preserves_completed_normalization() {
        for normalization in [
            "invert = invert or false",
            "if invert == nil then invert = false end",
        ] {
            let source = format!(
                r#"
                ---@param invert boolean?
                local function factory(invert)
                    {normalization}
                    ---@return boolean
                    local function callback() return invert end
                    return callback
                end
            "#
            );
            let mut ws = VirtualWorkspace::new();
            assert!(
                ws.has_no_diagnostic(DiagnosticCode::ReturnTypeMismatch, &source),
                "{source}"
            );
        }
    }

    #[test]
    fn test_mutable_capture_keeps_later_and_deferred_writes() {
        for source in [
            r#"
            ---@type boolean?
            local invert
            local function factory()
                invert = nil
                ---@return boolean
                local function callback() return invert end
                invert = false
                return callback
            end
            "#,
            r#"
            ---@param invert boolean?
            local function factory(invert)
                invert = invert or false
                ---@return boolean
                local function callback() return invert end
                invert = nil
                return callback
            end
            "#,
            r#"
            ---@param invert boolean?
            local function factory(invert)
                local function clear() invert = nil end
                invert = invert or false
                ---@return boolean
                local function callback() return invert end
                return callback, clear
            end
            "#,
            r#"
            ---@param invert boolean?
            local function factory(invert)
                local callbacks = {}
                for i = 1, 2 do
                    invert = nil
                    callbacks[i] = function() return invert end
                    invert = false
                    ---@return boolean
                    local function callback() return invert end
                    callbacks[i + 2] = callback
                end
                return callbacks
            end
            "#,
        ] {
            let mut ws = VirtualWorkspace::new();
            assert!(
                !ws.has_no_diagnostic(DiagnosticCode::ReturnTypeMismatch, source),
                "{source}"
            );
        }
    }

    #[test]
    fn test_mutable_capture_preserves_normalized_local() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ReturnTypeMismatch,
            r#"
            local function factory()
                ---@type boolean?
                local invert
                invert = false
                ---@return boolean
                local function callback() return invert end
                return callback
            end
        "#
        ));
    }

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

    fn mutable_capture_observed_type(source: &str) -> Option<crate::LuaType> {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(source);
        mutable_capture_observed_type_in_file(&ws, file)
    }

    fn mutable_capture_observed_type_in_file(
        ws: &VirtualWorkspace,
        file: crate::FileId,
    ) -> Option<crate::LuaType> {
        use emmylua_parser::{LuaAstNode, LuaAstToken, LuaLocalName};
        let model = ws.analysis.semantic_model(file);
        let locals = model
            .chunk()
            .unwrap()
            .descendants::<LuaLocalName>()
            .filter(|node| {
                node.get_name_token()
                    .is_some_and(|token| token.get_name_text() == "observed")
            })
            .collect::<Vec<_>>();
        assert_eq!(locals.len(), 1, "one observation local is required");
        let decl = model
            .decl_by_offset(locals[0].get_name_token().unwrap().get_position())
            .unwrap();
        model.type_of_decl(&decl)
    }

    #[test]
    fn test_mutable_capture_ignores_unreachable_writes() {
        let mut failures = Vec::new();
        for body in [
            "do return end; value = 1",
            "do return end; value = 1; value = 2",
            "if false then value = 1 end",
            "if nil then value = 1 end",
            "if not (true) then value = 1 end",
            "if true then return end; value = 1",
            "while false do value = 1 end",
            "goto done; value = 1; ::done::",
            "do return end; function value() end",
            "do return end; repeat value = 1 until true",
            "do return end; local function deferred() value = 1 end",
            "do return end; ::orphan:: value = 1",
        ] {
            let source = format!(
                "local value = 0; local function writer() {body} end; local function read() local observed = value end"
            );
            let actual = mutable_capture_observed_type(&source);
            if actual != Some(crate::LuaType::IntegerConst(0)) {
                failures.push(format!("{body}: {actual:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn test_mutable_capture_keeps_reachable_control_flow_writes() {
        let mut failures = Vec::new();
        for body in [
            "value = 1",
            "if unknown then value = 1 end",
            "if 0 then value = 1 end",
            "if '' then value = 1 end",
            "if not (false) then value = 1 end",
            "while unknown do value = 1; break end",
            "repeat value = 1 until unknown",
            "goto write; ::write:: value = 1",
            "if unknown then return end; value = 1",
        ] {
            let source = format!(
                "local value = 0; local function writer() {body} end; local function read() local observed = value end"
            );
            let actual = mutable_capture_observed_type(&source);
            let expected = crate::LuaType::from_vec(vec![
                crate::LuaType::IntegerConst(0),
                crate::LuaType::IntegerConst(1),
            ]);
            if actual != Some(expected) {
                failures.push(format!("{body}: {actual:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn test_mutable_capture_dead_nil_write_preserves_member_safety() {
        for (prefix, safe) in [("do return end;", true), ("", false)] {
            let mut ws = VirtualWorkspace::new();
            let source = format!(
                r#"
                local value = {{ name = 'ready' }}
                local function writer() {prefix} value = nil end
                local function read() return value.name end
            "#
            );
            assert_eq!(
                ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
                safe,
                "{prefix}"
            );
        }
    }

    #[test]
    fn test_mutable_capture_dead_writes_do_not_block_completed_assignment() {
        let mut failures = Vec::new();
        for body in [
            "do return end; value = 2",
            "if false then value = 2 end",
            "goto done; value = 2; ::done::",
            "do return end; local function deferred() value = 2 end",
        ] {
            let source = format!(
                "local value; value = 1; local function writer() {body} end; local function read() local observed = value end"
            );
            let actual = mutable_capture_observed_type(&source);
            if actual != Some(crate::LuaType::IntegerConst(1)) {
                failures.push(format!("{body}: {actual:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn test_mutable_capture_dead_branches_keep_live_alternatives() {
        let mut failures = Vec::new();
        for body in [
            "if false then value = 2 else value = 1 end",
            "goto live; value = 2; ::dead:: value = 2; ::live:: value = 1",
            "if true then value = 1 else value = 2 end",
            "if unknown then if false then value = 2 end; value = 1 else do return end; value = 2 end",
            "::again:: if unknown then value = 1; goto again end; if false then value = 2 end",
            "while unknown do if false then value = 2 else value = 1 end; break end",
        ] {
            let source = format!(
                "local value = 0; local function writer() {body} end; local function read() local observed = value end"
            );
            let actual = mutable_capture_observed_type(&source);
            let expected = crate::LuaType::from_vec(vec![
                crate::LuaType::IntegerConst(0),
                crate::LuaType::IntegerConst(1),
            ]);
            if actual != Some(expected) {
                failures.push(format!("{body}: {actual:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn test_mutable_capture_reachability_refreshes_after_file_edits() {
        let mut ws = VirtualWorkspace::new();
        let mut previous_file = None;
        for condition in ["false", "true", "false"] {
            let source = format!(
                "local value = 0; local function writer() if {condition} then value = 1 end end; local function read() local observed = value end"
            );
            let file = ws.def_file("capture.lua", &source);
            if let Some(previous) = previous_file {
                assert_eq!(file, previous);
            }
            previous_file = Some(file);
            let expected = if condition == "false" {
                crate::LuaType::IntegerConst(0)
            } else {
                crate::LuaType::from_vec(vec![
                    crate::LuaType::IntegerConst(0),
                    crate::LuaType::IntegerConst(1),
                ])
            };
            assert_eq!(
                mutable_capture_observed_type_in_file(&ws, file),
                Some(expected),
                "{condition}"
            );
        }
    }
}
