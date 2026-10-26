#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

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
}

#[test]
fn loop_carried_counter_does_not_stay_at_its_initializer() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        crate::DiagnosticCode::UnnecessaryIf,
        r#"
---@param stops string[]
local function report(stops)
    local count = 0
    for _, stop in pairs(stops) do
        if count % 10 == 0 then
            if count > 0 then print(count) end
        end
        if stop ~= '' then count = count + 1 end
    end
end
"#
    ));
}

#[test]
fn loop_carried_values_cover_all_loop_forms() {
    for body in [
        "for i = 1, 50 do if count > 0 then print(count) end; count = count + 1 end",
        "for _, value in pairs(values) do if count > 0 then print(count) end; count = count + 1 end",
        "while flag do if count > 0 then print(count) end; count = count + 1 end",
        "repeat if count > 0 then print(count) end; count = count + 1 until flag",
        "for i = 1, 10 do for j = 1, 10 do if count > 0 then print(count) end; count = count + 1 end end",
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            "---@param flag boolean\n---@param values string[]\nlocal function f(flag, values)\n local count = 0\n {body}\nend"
        );
        assert!(
            ws.has_no_diagnostic(crate::DiagnosticCode::UnnecessaryIf, &source),
            "{body}"
        );
    }
}

#[test]
fn loop_carried_values_keep_iteration_guards_and_constant_diagnostics() {
    for body in [
        "local count = 0; for i = 1, 10 do if count > 0 then print(count) end end",
        "for i = 1, 10 do local count = 0; if count > 0 then print(count) end; count = count + 1 end",
        "local count = 0; for i = 1, 10 do local count = 4; count = count + 1 end; if count > 0 then print(count) end",
        "local count = 0; for i = 1, 10 do count = 0; if count == 0 then print(count) end; count = count + 1 end",
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        assert!(
            !ws.has_no_diagnostic(crate::DiagnosticCode::UnnecessaryIf, body),
            "{body}"
        );
    }
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        crate::DiagnosticCode::NeedCheckNil,
        r#"
---@class Item
---@field name string
---@param items Item[]
local function f(items)
    local item = nil
    for _, next_item in pairs(items) do
        if item then print(item.name) end
        item = next_item
    end
end
"#
    ));
}

#[test]
fn loop_carried_counter_has_integer_type_and_optional_values_stay_optional() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def("local count = 0; for i = 1, 10 do if count > 0 then print(count) end; count = count + 1 end");
    let model = ws.analysis.semantic_model(file);
    let count = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| name.get_name_text().as_deref() == Some("count"))
        .unwrap();
    assert_eq!(
        model.type_of_expr_at(count.get_syntax_id(), count.get_position()),
        crate::LuaType::Integer
    );
    assert!(!ws.has_no_diagnostic(
        crate::DiagnosticCode::NeedCheckNil,
        r#"
---@class Item
---@field name string
---@param items Item[]
local function f(items)
    local item = nil
    for _, next_item in pairs(items) do
        print(item.name)
        item = next_item
    end
end
"#
    ));
}

#[test]
fn loop_carried_values_use_entry_state_and_return_slots() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for (source, name, expected) in [
        (
            "local count = 0; for i = 1, 10 do print(count); count = count + 1 end",
            "count",
            "integer",
        ),
        (
            "local value = nil; value = 'start'; for i = 1, 10 do print(value); value = value .. 'x' end",
            "value",
            "string",
        ),
        (
            "local function next_values() return 1, 's' end; local value = ''; for i = 1, 10 do print(value); local dummy; dummy, value = next_values() end",
            "value",
            "string",
        ),
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(source);
        let model = ws.analysis.semantic_model(file);
        let reference = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .find(|expr| {
                expr.get_name_text().as_deref() == Some(name)
                    && expr
                        .ancestors::<emmylua_parser::LuaCallExpr>()
                        .next()
                        .is_some()
            })
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(reference.get_syntax_id(), reference.get_position()),
            ws.ty(expected),
            "{source}"
        );
    }
}

#[test]
fn loop_carried_array_parameter_keeps_declared_element_type() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let source = r#"
---@class Item
---@field name string
---@class Result
---@field found boolean
---@field index integer?
---@return Result
local function search() end
---@param items Item[]
---@param limit integer?
local function f(items, limit)
    while (not limit or limit > 0) and #items > 0 do
        local result = search()
        if result.found then
            ---@cast result.index integer
            local item = assert(items[result.index])
            print(item.name)
            table.remove(items, result.index)
            if limit then limit = limit - 1 end
        else
            items = {}
        end
    end
end
"#;
    assert!(ws.has_no_diagnostic(crate::DiagnosticCode::ParamTypeMismatch, source));
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let items = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .filter(|name| name.get_name_text().as_deref() == Some("items"))
        .nth(1)
        .unwrap();
    let crate::LuaType::Array(array) =
        model.type_of_expr_at(items.get_syntax_id(), items.get_position())
    else {
        panic!("annotated parameter must retain its array contract");
    };
    assert_eq!(array.get_base(), &ws.ty("Item"));
    assert_eq!(array.get_len(), &crate::LuaArrayLen::Min(1));
}

#[test]
fn optional_named_fields_are_not_always_truthy() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        crate::DiagnosticCode::UnnecessaryIf,
        r#"
---@alias Id integer
---@class Event
---@field first? Id
---@field second? Id
---@param event Event
local function f(event)
    if event.first then print(event.first) end
    if event.second then print(event.second) end
end
"#
    ));
}
#[test]
fn required_named_fields_are_always_truthy() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(
        crate::DiagnosticCode::UnnecessaryIf,
        r#"
---@alias Id integer
---@class Event
---@field first Id
---@param event Event
local function f(event)
    if event.first then print(event.first) end
end
"#
    ));
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

#[test]
fn multiple_result_conditions_inspect_first_result() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        r#"
        ---@param stops table<integer, string>
        local function update(stops)
            if next(stops) then print('nonempty') end
        end
        ---@return boolean, string
        local function test() end
        if test() then print('yes') end
    "#
    ));
}

#[test]
fn multiple_result_conditions_keep_constant_first_results() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    for first in ["nil", "false", "true", "integer"] {
        let source = format!(
            "---@return {first}, string\nlocal function f() end\nif f() then print('branch') end"
        );
        assert!(
            !VirtualWorkspace::new_with_init_std_lib()
                .has_no_diagnostic(DiagnosticCode::UnnecessaryIf, &source),
            "{first}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (body, expected) in [
        (
            "local seen = {}; seen[key] = true; local observed = seen[key]",
            LuaType::BooleanConst(true),
        ),
        (
            "local seen = {}; if flag then seen[key] = true end; local observed = seen[key]",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
        (
            "local seen = {}; seen[other] = true; local observed = seen[key]",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
        (
            "local seen = {}; for i = 1, 3 do local observed = seen[key]; seen[key] = true end",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
        ("local seen = {}; local observed = seen[key]", LuaType::Nil),
        (
            "local seen = {}; local other = {}; local observed = seen[next_key(other)]",
            LuaType::Nil,
        ),
        (
            "for i = 1, 3 do local seen = {}; local observed = seen[key]; populate(seen) end",
            LuaType::Nil,
        ),
        (
            "local seen = {}; local other = {}; populate(other); local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "local seen = {}; print('unrelated'); seen[key] = true; local observed = seen[key]",
            LuaType::BooleanConst(true),
        ),
        (
            "local seen = {}; seen[key] = true; seen[key] = nil; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "for i = 1, 3 do local seen = {}; local observed = seen[key]; seen[key] = true end",
            LuaType::Nil,
        ),
        (
            "local seen = {}; local different = {}; different[key] = true; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "local seen = {}; seen[key] = true; seen = {}; local observed = seen[key]",
            LuaType::Nil,
        ),
    ] {
        let source = format!(
            r#"
        ---@param key string
        ---@param other string
        ---@param flag boolean
        local function probe(key, other, flag) {body} end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let pos = source.find("local observed = ").unwrap() + "local observed = ".len();
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|i| u32::from(i.get_position()) as usize == pos)
            .unwrap();
        let ty = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        assert_eq!(ty, expected, "{body}");
    }
}

#[test]
fn test_inferred_computed_table_writes_loop_condition() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        r#"
        ---@param keys string[]
        local function create(keys)
            local seen = {}
            for _, key in ipairs(keys) do
                ---@cast key string
                if seen[key] then print('duplicate') else seen[key] = true end
            end
        end
    "#
    ));
}

#[test]
fn test_inferred_computed_table_writes_key_and_result_slots() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (body, expected) in [
        (
            "local key = 'wanted'; local seen = {}; seen.other = true; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "local key = 'wanted'; local other = 'different'; local seen = {}; seen[other] = true; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "local seen = {}; seen[key] = pair(); local observed = seen[key]",
            LuaType::String,
        ),
        (
            "local seen = {}; local spare; seen[key], spare = pair(); local observed = seen[key]",
            LuaType::String,
        ),
        (
            "local seen = {}; local spare; spare, seen[key] = pair(); local observed = seen[key]",
            LuaType::Integer,
        ),
        (
            "local seen = {}; seen[key] = true; local alias = seen; seen = {}; local observed = alias[key]",
            LuaType::BooleanConst(true),
        ),
    ] {
        let source = format!(
            r#"
        ---@return string, integer
        local function pair() return 'value', 7 end
        ---@param key string
        local function probe(key) {body} end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let pos = source.find("local observed = ").unwrap() + "local observed = ".len();
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|i| u32::from(i.get_position()) as usize == pos)
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            expected,
            "{body}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes_finite_key_domains() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for key_type in ["'a'|'b'", "FiniteComputedKeys"] {
        let source = format!(
            r#"
        ---@alias FiniteComputedKeys 'a'|'b'
        ---@param key {key_type}
        local function probe(key)
            local seen = {{}}
            seen.other = true
            local observed = seen[key]
        end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            LuaType::Nil,
            "{key_type}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes_recursive_key_domains() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (alias, key_type) in [
        ("---@alias RecursiveKey 'a'|RecursiveKey", "RecursiveKey"),
        ("---@alias RecursiveKey RecursiveKey|'a'", "RecursiveKey"),
        (
            "---@alias GrowingKey<T> T|GrowingKey<T[]>",
            "GrowingKey<'a'>",
        ),
    ] {
        let source = format!(
            r#"
        {alias}
        ---@param key {key_type}
        local function probe(key)
            local seen = {{}}
            seen.other = true
            local observed = seen[key]
        end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        let ty = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        assert!(
            ty.is_unknown()
                || ty == LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
            "unresolved key domain must retain possible writes: {alias}: {ty:?}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes_written_key_domains() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (other_type, expected) in [
        ("'a'|'b'", LuaType::Nil),
        ("FiniteWrittenKeys", LuaType::Nil),
        ("integer", LuaType::Nil),
        (
            "RecursiveWrittenKey",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
    ] {
        let source = format!(
            r#"
        ---@alias FiniteWrittenKeys 'a'|'b'
        ---@alias RecursiveWrittenKey 'a'|RecursiveWrittenKey
        ---@param key 'wanted'
        ---@param other {other_type}
        local function probe(key, other)
            local seen = {{}}
            seen[other] = true
            local observed = seen[key]
        end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            expected,
            "{other_type}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes_open_empty_tables() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    let mut failures = Vec::new();
    for body in [
        "local seen = {}; populate(seen); local function probe(key) local observed = seen[key] end",
        "local parent = {seen = {}}; populate(parent.seen); local function probe(key) local observed = parent.seen[key] end",
        "local key = 'Foobar'; local seen = {}; populate(seen); local observed = seen[key]",
        "local key = 'Foobar'; local parent = {seen = {}}; populate(parent.seen); local observed = parent.seen[key]",
        "local key = 'Foobar'; local parent = {seen = {}}; populate(parent); local observed = parent.seen[key]",
        "local key = 'Foobar'; local seen = {}; local alias = seen; populate(alias); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; local result = populate(seen); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; if populate(seen) then end; local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; seen:populate(); local observed = seen[key]",
        "---@type table?\nlocal alias; local function fill() populate(alias) end; local seen = {}; alias = seen; fill(); local key = 'Foobar'; local observed = seen[key]",
        "local seen; local key = 'Foobar'; for i = 1, 2 do if not seen then seen = {} end; local observed = seen[key]; populate(seen) end",
        "local seen = {}; local key = 'Foobar'; ::again:: local observed = seen[key]; populate(seen); goto again",
        "local seen; local function fill() populate(seen) end; seen = {}; fill(); local key = 'Foobar'; local observed = seen[key]",
        "---@param value table\n---@return 'Foobar'\nlocal function populate_and_key(value) value.Foobar = true; return 'Foobar' end\nlocal seen = {}; local observed = seen[populate_and_key(seen)]",
        "---@param value table\n---@return 'Foobar'\nlocal function populate_and_key(value) value.Foobar = true; return 'Foobar' end\nlocal seen = {}; local alias = seen;\n---@cast alias table\nlocal observed = seen[populate_and_key(alias)]",
        "local key = 'Foobar'; local seen = {};\n---@type table\nlocal alias = seen; populate(alias); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; local alias = seen;\n---@cast alias table\npopulate(alias); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; local alias = seen; local second = alias;\n---@cast second table\npopulate(second); local observed = seen[key]",
        "---@type boolean\nlocal flag = ...; local key = 'Foobar'; local seen = {}; populate(flag and seen or nil); local observed = seen[key]",
        "---@type boolean\nlocal flag = ...; local key = 'Foobar'; local seen = {}; local box = {}; box.inner = flag and seen or nil; populate(box); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; for i = 1, 2 do local observed = seen[key]; populate(seen) end",
        "local key = 'Foobar'; local seen = {}; stored = seen; populate(); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; store.inner = seen; populate(store); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; populate({inner = seen}); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; local box = {inner = seen}; populate(box); local observed = seen[key]",
        "local key = 'Foobar'; local seen = {}; local function callback() populate(seen) end; callback(); local observed = seen[key]",
    ] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(body);
        let model = ws.analysis.semantic_model(file);
        let pos = body.find("local observed = ").unwrap() + "local observed = ".len();
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|index| u32::from(index.get_position()) as usize == pos)
            .unwrap();
        let ty = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        if ty != LuaType::Unknown {
            failures.push(format!("{body}: {ty:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
