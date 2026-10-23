#[cfg(test)]
mod test {
    use lsp_types::NumberOrString;
    use tokio_util::sync::CancellationToken;

    use crate::{DiagnosticCode, VirtualWorkspace};

    fn check_class_array_guard(element_type: &str, body: &str, expected_codes: &[DiagnosticCode]) {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let mut emmyrc = ws.get_emmyrc();
        emmyrc.strict.array_index = true;
        ws.update_emmyrc(emmyrc);
        let file_id = ws.def(&format!(
            r#"
            ---@class Container
            ---@field items {element_type}[]
            local _Container = {{}}

            ---@param t Container
            ---@param _other Container
            ---@param _index integer
            local function read(t, _other, _index)
                {body}
            end
            read({{ items = {{ 'hello' }} }}, {{ items = {{}} }}, 1)
            "#,
        ));
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .expect("diagnostics");
        let expected = expected_codes
            .iter()
            .map(|code| Some(NumberOrString::String(code.get_name().to_string())))
            .collect::<Vec<_>>();
        assert_eq!(
            diagnostics
                .iter()
                .map(|d| d.code.clone())
                .collect::<Vec<_>>(),
            expected,
            "{body}: {diagnostics:#?}"
        );
    }

    #[test]
    fn test_class_array_element_guards() {
        for body in [
            "if t.items[1] then print(t.items[1]:sub(1)) end",
            "if not t.items[1] then return end; print(t.items[1]:sub(1))",
            "if t.items[1] ~= nil then print(t.items[1]:sub(1)) end",
            "if t.items[1] == nil then return end; print(t.items[1]:sub(1))",
            "if t.items[1] then t.items[2] = 'other'; print(t.items[1]:sub(1)) end",
            "t.items = {}; if t.items[1] then print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[1] = t.items[1]:sub(1) end",
            "t.items[1] = t.items[2]; if t.items[1] then print(t.items[1]:sub(1)) end",
        ] {
            check_class_array_guard("string", body, &[]);
        }
    }

    #[test]
    fn test_class_array_element_guard_writes() {
        for body in [
            "if t.items[1] then t.items[1] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[1.0] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[1e0] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items = {}; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t = _other; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[_index] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then if _index == 1 then t.items = {} end; print(t.items[1]:sub(1)) end",
        ] {
            check_class_array_guard(
                "string",
                body,
                &[
                    DiagnosticCode::ParamTypeMismatch,
                    DiagnosticCode::NeedCheckNil,
                ],
            );
        }
    }

    #[test]
    fn test_class_array_element_guard_scope() {
        for body in [
            "print(t.items[1]:sub(1))",
            "if t.items[1] then print(t.items[2]:sub(1)) end",
            "if t.items[1] then end; print(t.items[1]:sub(1))",
            "if t.items[1] then print(_other.items[1]:sub(1)) end",
            "if t.items[1] then local function later() print(t.items[1]:sub(1)) end; t.items = {}; later() end",
            "if (t.items[1] ~= nil) ~= nil then print(t.items[1]:sub(1)) end",
            "if (not not t.items[1]) ~= nil then print(t.items[1]:sub(1)) end",
        ] {
            check_class_array_guard(
                "string",
                body,
                &[
                    DiagnosticCode::ParamTypeMismatch,
                    DiagnosticCode::NeedCheckNil,
                ],
            );
        }
        check_class_array_guard(
            "string",
            "if t.items[1] then local t = _other; print(t.items[1]:sub(1)) end",
            &[
                DiagnosticCode::RedefinedLocal,
                DiagnosticCode::ParamTypeMismatch,
                DiagnosticCode::NeedCheckNil,
            ],
        );
    }

    #[test]
    fn test_class_array_element_guard_preserves_assignment_type() {
        for (value, expected) in [
            ("'hello'", &[] as &[DiagnosticCode]),
            ("1", &[DiagnosticCode::AssignTypeMismatch]),
        ] {
            check_class_array_guard(
                "(string|number)",
                &format!(
                    r#"
                    t.items[1] = {value}
                    if t.items[1] then
                        ---@type string
                        local _s = t.items[1]
                    end
                    "#,
                ),
                expected,
            );
        }
    }

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
        "if rows[key] then mutate() local value = rows[key][other] end",
        "if rows[key] then local changed = mutate() local value = rows[key][other] end",
        "if rows[key] and mutate() then local value = rows[key][other] end",
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

fn check_global_collection_guard(body: &str, safe: bool) {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
---@class Entry
---@field label string
---@class Collection<K,V>: table<K,V>
---@alias Entries Collection<string, Entry?>
---@class Registry
---@field entries Entries
---@field array Entry[]
---@type Registry
registry = {}
---@type string
selected = ''
"#,
    );
    let source = format!(
        r#"
---@param key string
---@param other string
---@param flag boolean
---@param replacement Registry
---@param record {{id: string}}
local function label(key, other, flag, replacement, record)
    {body}
end
"#
    );
    assert_eq!(
        ws.has_no_diagnostic(crate::DiagnosticCode::NeedCheckNil, &source),
        safe,
        "{body}"
    );
}

#[test]
fn guarded_global_inherited_index_reads() {
    for body in [
        "if registry.entries[key] then refresh(); return registry.entries[key].label end",
        "if registry.entries[key] then local result = refresh(); return registry.entries[key].label end",
        "return registry.entries[key] and registry.entries[key].label or ''",
        "if registry.entries[key] then return registry.entries[key].label end",
        "if not registry.entries[key] then return end; return registry.entries[key].label",
        "local same = registry; return same.entries[key] and same.entries[key].label or ''",
        "return registry.entries[selected] and registry.entries[selected].label or ''",
        "return registry.entries[record.id] and registry.entries[record.id].label or ''",
        "return registry.entries['fixed'] and registry.entries['fixed'].label or ''",
        "if registry.array[1] then return registry.array[1].label end",
        "for i = 1, 10 do if registry.entries[key] then return registry.entries[key].label end end",
    ] {
        check_global_collection_guard(body, true);
    }
}

#[test]
fn guarded_global_index_facts_expire_on_mutations() {
    for body in [
        "return registry.entries[key].label",
        "if registry.entries[record.id] then record.id = other; return registry.entries[record.id].label end",
        "if registry.entries[key] then local _ENV = replacement; return registry.entries[key].label end",
        "return registry.entries[key] and registry.entries[other].label or ''",
        "if registry.entries[key] then key = other; return registry.entries[key].label end",
        "if registry.entries[selected] then selected = other; return registry.entries[selected].label end",
        "if registry.entries[key] then registry = replacement; return registry.entries[key].label end",
        "if registry.entries[key] then registry.entries[key] = nil; return registry.entries[key].label end",
        "if registry.entries[key] then local alias = registry.entries; alias[key] = nil; return registry.entries[key].label end",
        "if registry.entries[key] then local key = other; return registry.entries[key].label end",
        "if registry.entries[key] then local registry = replacement; return registry.entries[key].label end",
        "if registry.entries[key] then return function() return registry.entries[key].label end end",
    ] {
        check_global_collection_guard(body, false);
    }
}

#[test]
fn guarded_inherited_indexes_do_not_assume_pure_metamethods() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(
        crate::DiagnosticCode::NeedCheckNil,
        r#"
---@class Entry
---@field label string
---@class Changing: table<string, Entry?>
---@field __index fun(self: Changing, key: string): Entry?
---@param changing Changing
---@param key string
local function label(changing, key)
    return changing[key] and changing[key].label or ''
end
"#
    ));
}

#[cfg(test)]
mod equality_nullability_tests {
    use crate::{DiagnosticCode, VirtualWorkspace};

    fn check(required: bool, guard: &str, safe: bool) {
        let mut ws = VirtualWorkspace::new();
        let existing = if required { "Distance" } else { "Distance?" };
        let source = format!(
            r#"
---@class Distance
---@field value number?
---@param existing {existing}
---@param maybe Distance?
---@param enabled boolean
local function probe(existing, maybe, enabled)
    {guard}
end
"#
        );
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            safe,
            "{guard}"
        );
    }

    #[test]
    fn equality_nullability_required_parameter_survives_equivalent_guards() {
        for guard in [
            "if maybe ~= existing then return end; existing.value = 1",
            "if existing ~= maybe then return end; return existing.value",
            "if maybe ~= existing then else return existing.value end",
            "if not (maybe == existing) then return end; return existing.value",
            "if maybe == existing then return existing.value end",
            "if not enabled or maybe ~= existing then return end; return existing.value",
        ] {
            check(true, guard, true);
        }
    }

    #[test]
    fn equality_nullability_optional_parameters_can_both_be_nil() {
        for guard in [
            "if maybe ~= existing then return end; return existing.value",
            "if existing == maybe then return existing.value end",
        ] {
            check(false, guard, false);
        }
    }

    #[test]
    fn equality_nullability_inequality_does_not_prove_presence() {
        check(
            false,
            "if maybe ~= existing then return existing.value end",
            false,
        );
        check(
            false,
            "if maybe == existing then return end; return existing.value",
            false,
        );
    }
}

#[test]
fn guarded_optional_reference_field_after_many_loop_merges() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let mut source = String::from(
        r#"
---@class SignalValue
---@field name string
---@class Signal
---@field value? SignalValue
---@param values boolean[]
---@param signal Signal?
local function f(values, signal)
"#,
    );
    for _ in 0..12 {
        source.push_str("for _, value in pairs(values) do print(value) end\n");
    }
    source.push_str("local color = (signal and signal.value and signal.value.name == 'blue') and 'blue' or 'yellow'\nend");
    assert!(ws.has_no_diagnostic(crate::DiagnosticCode::NeedCheckNil, &source));
    let unguarded = source.replace("signal.value and ", "");
    assert!(!ws.has_no_diagnostic(crate::DiagnosticCode::NeedCheckNil, &unguarded));
}

#[test]
fn nonempty_length_guard_survives_unrelated_predicate_calls() {
    for condition in [
        "#a > 0",
        "a and #a > 0 and count() > 0",
        "count() > 0 and a and #a > 0",
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        assert!(
            ws.has_no_diagnostic(
                crate::DiagnosticCode::ParamTypeMismatch,
                &format!(
                    r#"
            ---@return integer
            local function count() return 1 end
            ---@param key string
            local function consume(key) end
            ---@param a string[]
            local function drain(a)
                while {condition} do
                    local key = a[#a]
                    consume(key)
                    table.remove(a)
                end
            end
        "#
                )
            ),
            "{condition}"
        );
    }
}

#[test]
fn nonempty_length_guard_rejects_mutating_predicates() {
    for mutation in [
        "table.remove(a)",
        "rawset(a, 1, nil)",
        "mutate(a)",
        "change()",
        "(function() a[1] = nil end)()",
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        assert!(
            !ws.has_no_diagnostic(
                crate::DiagnosticCode::ParamTypeMismatch,
                &format!(
                    r#"
            ---@param key string
            local function consume(key) end
            ---@param a string[]
            local function drain(a)
                local function change() a[1] = nil end
                if #a > 0 then
                    {mutation}
                    consume(a[#a])
                end
            end
        "#
                )
            ),
            "{mutation}"
        );
    }
}

#[test]
fn test_member_primitive_guards() {
    use crate::{DiagnosticCode, LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for body in [
        "if type(elem.handler) == 'table' then local observed = elem.handler; print(elem.handler.name) end",
        "if 'table' == type(elem.handler) then local observed = elem.handler; print(observed.name) end",
        "if type(elem['handler']) ~= 'table' then return end; local observed = elem.handler; print(observed.name)",
        "if not (type(elem.handler) ~= 'table') then local observed = elem.handler; print(observed.name) end",
        "if type(elem.handler) == 'table' then else return end; local observed = elem.handler; print(observed.name)",
        "if type(elem.handler) == 'string' then return end; if elem.handler ~= nil then local observed = elem.handler; print(observed.name) end",
    ] {
        let source = format!(
            r#"
        ---@class PrimitiveHandler
        ---@field name string
        local Handler = {{}}
        ---@class PrimitiveElement
        ---@field handler PrimitiveHandler|string|nil
        local Element = {{}}
        ---@param elem PrimitiveElement
        local function read(elem) {body} end
        "#
        );
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            "{body}"
        );
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let pos = source.rfind("elem.handler").unwrap() as u32;
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|i| {
                u32::from(i.get_position()) == pos
                    && i.get_range().len() == rowan::TextSize::from("elem.handler".len() as u32)
            })
            .unwrap();
        let ty = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        assert!(
            matches!(&ty, LuaType::Ref(id) if id.get_name() == "PrimitiveHandler"),
            "{body}: {ty:?}"
        );
    }
}

#[test]
fn test_member_primitive_guards_preserve_invalidations() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    for body in [
        "print(elem.handler.name)",
        "if type(other.handler) == 'table' then print(elem.handler.name) end",
        "if type(elem.handler) == 'table' then elem.handler = nil; print(elem.handler.name) end",
        "local type = function(_) return 'table' end; if type(elem.handler) == 'table' then print(elem.handler.name) end",
        "if type(elem.handler) == 'nil' then print(elem.handler.name) end",
    ] {
        let source = format!(
            r#"
        ---@class PrimitiveNegativeHandler
        ---@field name string
        local Handler = {{}}
        ---@class PrimitiveNegativeElement
        ---@field handler PrimitiveNegativeHandler?
        local Element = {{}}
        ---@param elem PrimitiveNegativeElement
        ---@param other PrimitiveNegativeElement
        local function read(elem, other) {body} end
        "#
        );
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(
            !ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            "{body}"
        );
    }
}

#[test]
fn test_member_primitive_guards_filter_other_primitives() {
    use crate::{DiagnosticCode, LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (guard, expected) in [
        ("type(elem.value) == 'string'", LuaType::String),
        ("'number' == type(elem.value)", LuaType::Integer),
        (
            "type(elem.value) ~= 'string' and type(elem.value) ~= 'nil'",
            LuaType::Integer,
        ),
    ] {
        let source = format!(
            r#"
        ---@class PrimitiveValues
        ---@field value string|integer|nil
        local Element = {{}}
        ---@param elem PrimitiveValues
        local function read(elem)
            if {guard} then local observed = elem.value end
        end
        "#
        );
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(
            ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, &source),
            "{guard}"
        );
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
            "{guard}"
        );
    }
}
