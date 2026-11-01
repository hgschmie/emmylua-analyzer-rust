//! Loop-carried inference, diagnostic boundaries, and shared-cache identity.

use crate::LuaType;

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
            ws.has_diagnostic(crate::DiagnosticCode::UnnecessaryIf, body),
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
        LuaType::Integer
    );
    assert!(ws.has_diagnostic(
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
    let LuaType::Array(array) = model.type_of_expr_at(items.get_syntax_id(), items.get_position())
    else {
        panic!("annotated parameter must retain its array contract");
    };
    assert_eq!(array.get_base(), &ws.ty("Item"));
    assert_eq!(array.get_len(), &crate::LuaArrayLen::Min(1));
}

#[test]
fn loop_cache_keeps_file_view_identity() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let source = "local value = 1\nwhile flag do\n local observed = value\n value = 'text'\nend";
    let a = ws.def_file("loop-a.lua", source);
    let b = ws.def_file("loop-b.lua", source);
    let model_a = ws.analysis.semantic_model(a);
    let read = model_a
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| name.get_name_text().as_deref() == Some("value"))
        .unwrap();
    let decl = model_a.resolve_name(read.get_position()).unwrap();
    let expected = model_a.type_of_decl_at(&decl, read.get_position());
    assert_eq!(
        expected,
        LuaType::from_vec(vec![LuaType::Integer, LuaType::String])
    );
    for foreign_first in [true, false] {
        let model_b = ws.analysis.semantic_model(b);
        let shared_a = model_b.model_for(a);
        if foreign_first {
            assert_eq!(
                model_b.type_of_decl_at(&decl, read.get_position()),
                LuaType::IntegerConst(1)
            );
        }
        assert_eq!(
            shared_a.type_of_decl_at(&decl, read.get_position()),
            expected
        );
        assert_eq!(
            model_b.type_of_decl_at(&decl, read.get_position()),
            LuaType::IntegerConst(1)
        );
        assert_eq!(
            shared_a.type_of_decl_at(&decl, read.get_position()),
            expected
        );
    }
}

#[test]
fn loop_cycle_seed_propagates_incompleteness() {
    use super::{CacheEntry, loop_carried_type};
    use emmylua_parser::{LuaAstNode, LuaNameExpr, LuaWhileStat};
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local value = 1; while flag do local read = value; value = value + 1 end");
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let loop_stat = chunk.descendants::<LuaWhileStat>().next().unwrap();
    let read = chunk
        .descendants::<LuaNameExpr>()
        .find(|name| name.get_name_text().as_deref() == Some("value"))
        .unwrap();
    let decl = model.resolve_name(read.get_position()).unwrap();
    let syntax = loop_stat.get_syntax_id();
    let entry = model
        .flow_tree()
        .unwrap()
        .get_flow_id_at(read.get_position())
        .unwrap();
    model
        .cache
        .borrow_mut()
        .loop_carried
        .insert((file, syntax, decl.clone()), CacheEntry::InProgress);
    let revision = model.cache.borrow().revision();
    assert_eq!(
        loop_carried_type(&model, &decl, syntax, entry),
        Some(LuaType::Integer)
    );
    assert!(
        !model.cache.borrow().is_complete(revision),
        "a cycle seed must not make its dependent query eligible for a Ready result"
    );
}

#[test]
fn loop_local_shadow_does_not_inherit_parameter_annotation() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(
        r#"
---@param value integer
local function update(value)
    do
        local value = 1
        while flag do
            local observed = value
            value = 'text'
        end
    end
end
"#,
    );
    let model = ws.analysis.semantic_model(file);
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| name.get_name_text().as_deref() == Some("value"))
        .unwrap();
    assert_eq!(
        model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
        LuaType::from_vec(vec![LuaType::Integer, LuaType::String])
    );
}

#[test]
fn loop_generic_integer_seed_does_not_hide_noninteger_writes() {
    for (rhs, other_write) in [
        ("identity(index + 0.5)", ""),
        ("identity(index)", "index = 0.5"),
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        assert!(
            ws.has_diagnostic(
                crate::DiagnosticCode::AssignTypeMismatch,
                &format!(
                    r#"
            ---@generic T
            ---@param value T
            ---@return T
            local function identity(value) return value end
            ---@param initial integer
            ---@param items integer[]
            local function update(initial, items)
                local index = initial
                for _, item in ipairs(items) do
                    index = {rhs}
                    {other_write}
                end
                ---@type integer
                local result = index
            end
        "#
                )
            ),
            "{rhs}"
        );
    }
}

#[test]
fn loop_continue_and_goto_paths_keep_pre_loop_seed() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for tail in ["continue", "if flag then goto again end"] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            "local value = 1; while flag do ::again:: local observed = value; value = 'text'; {tail} end"
        ));
        let model = ws.analysis.semantic_model(file);
        let read = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .find(|name| name.get_name_text().as_deref() == Some("value"))
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
            LuaType::from_vec(vec![LuaType::Integer, LuaType::String]),
            "{tail}"
        );
    }
}

#[test]
fn while_condition_still_narrows_loop_carried_optional_values() {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        crate::DiagnosticCode::NeedCheckNil,
        r#"
---@class LoopItem
---@field name string
---@return LoopItem?
local function next_item() end
---@param item LoopItem?
local function consume(item)
    while item ~= nil do
        print(item.name)
        item = next_item()
    end
end
"#
    ));
}

#[test]
fn repeat_condition_reads_loop_carried_values() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(
        r#"
---@return string
local function next_text() end
---@param flag boolean
local function consume(flag)
    local value = 0
    repeat
        if flag then value = next_text() end
    until value == 'done'
end
"#,
    );
    let model = ws.analysis.semantic_model(file);
    let condition_read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .filter(|name| name.get_name_text().as_deref() == Some("value"))
        .last()
        .unwrap();
    assert_eq!(
        model.type_of_expr_at(
            condition_read.get_syntax_id(),
            condition_read.get_position()
        ),
        LuaType::from_vec(vec![LuaType::Integer, LuaType::String])
    );
}

#[test]
fn loop_parameter_contract_uses_doc_name_not_doc_order() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(
        r#"
---@param other string
---@param value integer
local function consume(value, other)
    while flag do
        local observed = value
        value = 'invalid'
    end
end
"#,
    );
    let model = ws.analysis.semantic_model(file);
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| name.get_name_text().as_deref() == Some("value"))
        .unwrap();
    assert_eq!(
        model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
        LuaType::Integer
    );
}

#[test]
fn loop_rhs_dependency_cache_separates_file_views() {
    use emmylua_parser::{LuaAstNode, LuaBinaryExpr, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let source = "local value = 1; while flag do value = value + 1 end";
    let a = ws.def_file("dependency-a.lua", source);
    let b = ws.def_file("dependency-b.lua", source);
    let owner = ws.analysis.semantic_model(a);
    let rhs = owner
        .chunk()
        .unwrap()
        .descendants::<LuaBinaryExpr>()
        .next()
        .unwrap();
    let read = rhs.descendants::<LuaNameExpr>().next().unwrap();
    let decl = owner.resolve_name(read.get_position()).unwrap();
    for foreign_first in [true, false] {
        let shared = ws.analysis.semantic_model(a);
        let foreign = shared.model_for(b);
        if foreign_first {
            assert!(!super::references_decl(
                &foreign,
                rhs.get_syntax_id(),
                &decl
            ));
        }
        assert!(super::references_decl(&shared, rhs.get_syntax_id(), &decl));
        assert!(!super::references_decl(
            &foreign,
            rhs.get_syntax_id(),
            &decl
        ));
        assert!(super::references_decl(&shared, rhs.get_syntax_id(), &decl));
    }
}
