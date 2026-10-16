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
