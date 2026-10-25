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
fn test_computed_primitive_guards() {
    use crate::{DiagnosticCode, LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for body in [
        "if type(rows[key]) == 'table' then local observed = rows[key]; print(observed.name) end",
        "if type(rows[key]) == 'table' then local label = type(key); local text = tostring(key); local observed = rows[key]; print(observed.name) end",
        "if type(rows[key]) ~= 'string' then if type(rows[key]) ~= 'nil' then local observed = rows[key]; print(observed.name) end end",
        "if type(rows[key]) == 'string' then return end; if type(rows[key]) == 'nil' then return end; local observed = rows[key]; print(observed.name)",
        "if type(rows[key]) ~= 'string' then if rows[key] then local observed = rows[key]; print(observed.name) end end",
        "if 'table' == type(rows[key]) then local observed = rows[key]; print(observed.name) end",
        "if type(rows[key]) ~= 'table' then return end; local observed = rows[key]; print(observed.name)",
        "if not (type(rows[key]) == 'table') then return end; local observed = rows[key]; print(observed.name)",
    ] {
        let source = format!(
            r#"
        ---@param rows (table|string|nil)[]
        ---@param key integer
        local function read(rows, key) {body} end
        "#
        );
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            "{body}"
        );
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let pos = source.rfind("rows[key]").unwrap();
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|i| u32::from(i.get_position()) as usize == pos)
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            LuaType::Table,
            "{body}"
        );
    }
}

#[test]
fn test_computed_primitive_guards_keep_effect_boundaries() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    for body in [
        "if type(rows[key]) == 'table' then key = other; print(rows[key].name) end",
        "if type(rows[key]) == 'table' then rows[key] = nil; print(rows[key].name) end",
        "if type(rows[key]) == 'table' then rows = {}; print(rows[key].name) end",
        "if type(rows[key]) == 'table' then mutate(); print(rows[key].name) end",
        "if type(rows[key]) == 'table' then local changed = mutate(); print(rows[key].name) end",
        "if type(rows[other]) == 'table' then print(rows[key].name) end",
        "local type = function(_) return 'table' end; if type(rows[key]) == 'table' then print(rows[key].name) end",
    ] {
        let source = format!(
            r#"
        ---@param rows (table|string|nil)[]
        ---@param key integer
        ---@param other integer
        local function read(rows, key, other)
            local function mutate() rows[key] = nil end
            {body}
        end
        "#
        );
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(
            !ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            "{body}"
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

#[test]
fn test_computed_primitive_join_paths_keep_incoming_filters() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (body, expected) in [
        (
            "if flag then if type(rows[key]) ~= 'table' then return end else if type(rows[key]) == 'string' then return end; if not rows[key] then return end end; local observed = rows[key]",
            LuaType::Table,
        ),
        (
            "if flag then if type(rows[key]) ~= 'string' then return end else if type(rows[key]) ~= 'table' then return end end; local observed = rows[key]",
            LuaType::from_vec(vec![LuaType::Table, LuaType::String]),
        ),
        (
            "if flag then if type(rows[key]) ~= 'table' then return end end; local observed = rows[key]",
            LuaType::from_vec(vec![LuaType::Table, LuaType::String, LuaType::Nil]),
        ),
    ] {
        let source = format!(
            "---@param rows (table|string|nil)[]\n---@param key integer\n---@param flag boolean\nlocal function read(rows, key, flag) {body} end"
        );
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let offset = source.rfind("rows[key]").unwrap();
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|index| u32::from(index.get_position()) as usize == offset)
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            expected,
            "{body}"
        );
    }
}
