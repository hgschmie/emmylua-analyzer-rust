#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_issue_245() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        local a --- @type table?
        local _ = (a and a.type == 'change') and a.field
        "#
        ));
    }
    #[test]
    fn test_issue_402() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class A
            local a = {}

            ---@param self table?
            function a.new(self)
                if self then
                    self.a = 1
                end
            end
        "#
        ));
    }

    #[test]
    fn test_issue_474() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Range4
            ---@class TSNode: userdata
            ---@field range fun(self: TSNode): Range4

            ---@param node_or_range TSNode|Range4
            ---@return Range4
            function foo(node_or_range)
                if type(node_or_range) == 'table' then
                    return node_or_range
                else
                    return node_or_range:range()
                end
            end
            "#
        ));
    }

    #[test]
    fn test_cast() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class Cast1
            ---@field get fun(self: self, a: number): Cast1?
            ---@field get2 fun(self: self, a: number): Cast1?
        "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
                ---@type Cast1
                local A

                local a = A:get(1) --[[@cast -?]]
                    :get2(2)
            "#
        ));
    }

    #[test]
    fn test_issue_895_891() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        local t = {
        123,
        234,
        345,
        }

        ---@param id number
        function test(id) end

        for i = 1, #t do
            test(t[i]) -- expected 'number' but found (123|234|345)?
        end
        "#,
        ));
    }

    #[test]
    fn test_issue_886() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            r#"
        ---@type string[]
        local a = {}

        -- if #a == 0 then return end
        if not a[1] then return end

        -- ---@type string
        -- local s = a[1]

        ---@type string
        local s = a[#a]
        "#,
        ));
    }

    #[test]
    fn test_asserted_index_assignment_prefix_is_not_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        ---@type integer[][]
        local res
        res[1][1] = assert(res[1])[2]
        "#,
        ));
    }

    #[test]
    fn test_numeric_for_len_expr_narrows_loop_body_value_to_non_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        ---@type false|fun(...)[]?
        local calls

        for i = 1, #calls do
            calls[i](...)
        end
        "#,
        ));
    }

    #[test]
    fn test_different_asserted_index_assignment_prefix_still_needs_nil_check() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        ---@type integer[][]
        local res
        res[1][1] = assert(res[2])[2]
        "#,
        ));
    }
}

#[test]
fn test_computed_guards() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    for body in [
        "if rows[key] and rows[key][other] then local value = rows[key][other] end",
        "if rows[key] ~= nil then local value = rows[key][other] end",
        "if nil ~= rows[key] then local value = rows[key][other] end",
        "if not rows[key] then return end local value = rows[key][other]",
        "if rows[key] == nil then return end local value = rows[key][other]",
        "if rows[key] then local unrelated = 1 local value = rows[key][other] end",
        "if rows[key] then local deferred = function() mutate() end local value = rows[key][other] end",
    ] {
        for library in [false, true] {
            let mut ws = if library {
                VirtualWorkspace::new_with_init_std_lib()
            } else {
                VirtualWorkspace::new()
            };
            assert!(
                ws.has_no_diagnostic(
                    DiagnosticCode::NeedCheckNil,
                    &format!(
                        r#"
---@param rows integer[][]
---@param key integer
---@param other integer
local function read(rows, key, other)
    {body}
end
"#
                    )
                ),
                "{body} (stdlib: {library})"
            );
        }
    }
}

#[test]
fn test_computed_guards_do_not_cross_mutations() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    for body in [
        "if rows[key] then key = nextkey local value = rows[key][other] end",
        "if rows[key] then rows = replacement local value = rows[key][other] end",
        "if rows[key] then rows[key] = nil local value = rows[key][other] end",
        "if rows[key] then alias[key] = nil local value = rows[key][other] end",
        "if rows[key] then local key = nextkey local value = rows[key][other] end",
        "if rows[key] then mutate() local value = rows[key][other] end",
        "if rows[key] then local changed = mutate() local value = rows[key][other] end",
        "if rows[key] and mutate() then local value = rows[key][other] end",
        "if rows[key] then local deferred = function() local value = rows[key][other] end end",
        "if rows[key] then end local value = rows[key][other]",
        "if rows[key] or rows[key][other] then end",
        "if rows[key] == nil then local value = rows[key][other] end",
        "if rows[nextkey] then local value = rows[key][other] end",
    ] {
        let mut ws = VirtualWorkspace::new();
        assert!(
            !ws.has_no_diagnostic(
                DiagnosticCode::NeedCheckNil,
                &format!(
                    r#"
---@param rows integer[][]
---@param replacement integer[][]
---@param key integer
---@param nextkey integer
---@param other integer
local function read(rows, replacement, key, nextkey, other)
    local alias = rows
    {body}
end
"#
                )
            ),
            "{body}"
        );
    }
}

#[test]
fn test_nested_computed_guards() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::NeedCheckNil,
        r#"
---@class Dispatcher
---@field Provided integer[][]
---@param dispatcher Dispatcher
---@param key integer
---@param other integer
local function read(dispatcher, key, other)
    if dispatcher.Provided[key] and dispatcher.Provided[key][other] then
        local count = dispatcher.Provided[key][other]
    end
end
"#
    ));
    let mut ws = VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::NeedCheckNil,
        r#"
---@param rows integer[][]
---@param key integer
local function read(rows, key)
    if rows[key] then local first = rows[key][1] end
end
"#
    ));
}

#[test]
fn test_nonempty_array_length_index() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    for (guard, index, mutation, expected_warning) in [
        ("if not a[1] then return end", "a[#a]", "", false),
        ("if #a > 0 then", "a[#a]", "", false),
        ("if #a >= 1 then", "a[#a]", "", false),
        ("if #a == 1 then", "a[#a]", "", false),
        ("if #a <= 0 then return end", "a[#a]", "", false),
        ("", "a[#a]", "", true),
        ("if #a <= 1 then", "a[#a]", "", true),
        ("if 1 >= #a then", "a[#a]", "", true),
        ("if 1 > #a then", "a[#a]", "", true),
        ("if #a == 0 then", "a[#a]", "", true),
        ("if a[1] then return end", "a[#a]", "", true),
        ("if not a[1] then return end", "b[#b]", "", true),
        ("if not a[1] then return end", "a[#b]", "", true),
        ("if not a[1] then return end", "a[#a]", "a[1] = nil", true),
        ("if not a[1] then return end", "a[#a]", "a = {}", true),
        (
            "if not a[1] then return end",
            "a[#a]",
            "local alias = a; alias[1] = nil",
            true,
        ),
        ("if not a[1] then return end", "a[#a]", "mutate(a)", true),
        (
            "if not a[1] then return end",
            "a[#a]",
            "local ignored = mutate(a)",
            true,
        ),
    ] {
        let closing = if guard.ends_with("then") { "end" } else { "" };
        let source = format!(
            r#"
            ---@param a string[]
            ---@param b string[]
            function example(a, b)
                {guard}
                {mutation}
                ---@type string
                local s = {index}
                {closing}
            end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
            !expected_warning,
            "{source}"
        );
    }
}
