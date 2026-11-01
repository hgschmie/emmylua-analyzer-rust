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
    let source = "local count = 0; for i = 1, 10 do if count > 0 then print(count) end; count = count + 1 end";
    let file = ws.def(source);
    let offset = source.find("if count").unwrap() + "if ".len();
    let model = ws.analysis.semantic_model(file);
    let count = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| u32::from(name.get_position()) as usize == offset)
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
    let offset = source.find("assert(items[").unwrap() + "assert(".len();
    let items = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| u32::from(name.get_position()) as usize == offset)
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
    let tree = model.flow_tree().unwrap();
    let loop_flow = (0..tree.node_count())
        .map(crate::semantic_db::flow::FlowId)
        .find(|flow| {
            tree.get_flow_effects(*flow).iter().any(|effect| {
                matches!(effect, crate::semantic_db::flow::FlowEffect::LoopEntry(id) if *id == syntax)
            })
        })
        .unwrap();
    let entry = super::super::first_antecedent(
        &tree,
        tree.get_flow_node(loop_flow).unwrap().antecedent.as_ref(),
    )
    .expect("the loop has a pre-loop predecessor before any continue edge");
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
        let foreign_rhs = foreign
            .chunk()
            .unwrap()
            .descendants::<LuaBinaryExpr>()
            .next()
            .unwrap();
        assert_eq!(foreign_rhs.get_syntax_id(), rhs.get_syntax_id());
        assert_ne!(
            foreign.resolve_name(read.get_position()),
            Some(decl.clone())
        );
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

#[test]
fn loop_carried_conditional_self_updates_keep_integer_read_type() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for update in [
        "if flag then count = count + 1 end",
        "if flag then count = count + 1 else count = 0 end",
        "if flag then count = 1 end; count = count + 1",
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            "local count = 0; for i = 1, 10 do {update}; local observed = count end"
        ));
        let model = ws.analysis.semantic_model(file);
        let read = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .filter(|name| name.get_name_text().as_deref() == Some("count"))
            .last()
            .unwrap();
        for phase in ["cold", "warm"] {
            assert_eq!(
                model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
                LuaType::Integer,
                "{update}, {phase}"
            );
        }
    }
}

#[test]
fn loop_parameter_contract_does_not_hide_direct_invalid_write() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let source = r#"
---@param value integer
local function consume(value)
    while flag do
        local before_write = value
        value = nil
        local after_write = value
    end
end
"#;
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    let reads: Vec<_> = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .filter(|name| name.get_name_text().as_deref() == Some("value"))
        .collect();
    assert_eq!(reads.len(), 3);
    for phase in ["cold", "warm"] {
        assert_eq!(
            model.type_of_expr_at(reads[0].get_syntax_id(), reads[0].get_position()),
            LuaType::Integer,
            "entry contract, {phase}"
        );
        assert_eq!(
            model.type_of_expr_at(reads[2].get_syntax_id(), reads[2].get_position()),
            LuaType::Nil,
            "direct violating assignment, {phase}"
        );
    }
    assert!(ws.has_diagnostic(crate::DiagnosticCode::AssignTypeMismatch, source));
}

#[test]
fn loop_carried_function_statement_replaces_an_outer_local() {
    use emmylua_parser::{LuaAstNode, LuaFuncStat, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def(
        "local action = nil; while flag do local observed = action; function action() end end",
    );
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let read = chunk
        .descendants::<LuaNameExpr>()
        .find(|name| name.get_name_text().as_deref() == Some("action"))
        .unwrap();
    let closure = chunk
        .descendants::<LuaFuncStat>()
        .next()
        .unwrap()
        .get_closure()
        .unwrap();
    let expected = LuaType::from_vec(vec![
        LuaType::Nil,
        model.type_of_expr(closure.get_syntax_id()),
    ]);
    assert_ne!(expected, LuaType::Nil);
    for phase in ["cold", "warm"] {
        assert_eq!(
            model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
            expected,
            "{phase}"
        );
    }
}

#[test]
fn loop_function_statements_keep_local_and_closure_boundaries() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for source in [
        "while flag do local action = nil; local observed = action; function action() end end",
        "local action = nil; while flag do local observed = action; local function unused() function action() end end end",
        "local action = nil; local receiver = {}; while flag do local observed = action; function receiver.action() end end",
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(source);
        let model = ws.analysis.semantic_model(file);
        let read = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .find(|name| name.get_name_text().as_deref() == Some("action"))
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
            LuaType::Nil,
            "{source}"
        );
    }
}

#[test]
fn loop_rhs_dependency_cache_is_fresh_after_a_file_edit() {
    use emmylua_parser::{LuaAstNode, LuaBinaryExpr, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let source = "local value = 1; local other = 0; while flag do value = value + 1 end";
    let file = ws.def_file("edited-dependency.lua", source);
    let (syntax, decl) = {
        let model = ws.analysis.semantic_model(file);
        let rhs = model
            .chunk()
            .unwrap()
            .descendants::<LuaBinaryExpr>()
            .next()
            .unwrap();
        let read = rhs.descendants::<LuaNameExpr>().next().unwrap();
        let decl = model.resolve_name(read.get_position()).unwrap();
        assert!(super::references_decl(&model, rhs.get_syntax_id(), &decl));
        (rhs.get_syntax_id(), decl)
    };
    assert_eq!(
        ws.def_file(
            "edited-dependency.lua",
            &source.replace("value + 1", "other + 1")
        ),
        file
    );
    let model = ws.analysis.semantic_model(file);
    let rhs = model
        .chunk()
        .unwrap()
        .descendants::<LuaBinaryExpr>()
        .next()
        .unwrap();
    assert_eq!(rhs.get_syntax_id(), syntax);
    assert!(model.cache.borrow().loop_rhs_dependencies.is_empty());
    assert!(!super::references_decl(&model, syntax, &decl));
    assert!(!super::references_decl(&model, syntax, &decl));
}

#[test]
fn loop_globals_keep_values_from_prior_iterations() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let source = "while flag do global_value = global_value or 0; local observed = global_value; global_value = 'text' end";
    let file = ws.def(source);
    // The recursive global initializer has an incomplete entry value. Its
    // unknown seed must not hide the independent write on a later iteration.
    let expected = LuaType::from_vec(vec![LuaType::Unknown, LuaType::String]);
    let model = ws.analysis.semantic_model(file);
    let read_offset = source.find("local observed = ").unwrap() + "local observed = ".len();
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| u32::from(name.get_position()) as usize == read_offset)
        .unwrap();
    for phase in ["cold", "warm"] {
        assert_eq!(
            model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
            expected,
            "{phase}"
        );
    }
}

#[test]
fn loop_carried_types_exclude_statically_unreachable_writes() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for tail in [
        "break; value = 'text'",
        "return; value = 'text'",
        "if false then value = 'text' end",
        "if not true then value = 'text' end",
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!("local function run() local value = 1; while flag do local observed = value; {tail} end end"));
        let model = ws.analysis.semantic_model(file);
        let read = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .find(|name| name.get_name_text().as_deref() == Some("value"))
            .unwrap();
        for phase in ["cold", "warm"] {
            let actual = model.type_of_expr_at(read.get_syntax_id(), read.get_position());
            assert!(
                matches!(actual, LuaType::Integer | LuaType::IntegerConst(1)),
                "{tail}: {phase}: {actual:?}"
            );
        }
    }
}

#[test]
fn loop_rhs_dependency_distinguishes_closure_values_from_possible_calls() {
    use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaVarExpr};
    for (rhs, references) in [
        ("function() return value end", false),
        ("{get = function() return value end}", false),
        (
            "(function() return function() return value end end)()",
            false,
        ),
        ("(function() return value end)()", true),
        (
            "(function() return (function() return value end)() end)()",
            true,
        ),
        ("fn(function() return value end)", true),
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!(
            "local function run(fn) local value = 1; while flag do value = {rhs} end end"
        ));
        let model = ws.analysis.semantic_model(file);
        let assignment = model
            .chunk()
            .unwrap()
            .descendants::<LuaAssignStat>()
            .next()
            .unwrap();
        let (vars, values) = assignment.get_var_and_expr_list();
        let LuaVarExpr::NameExpr(target) = &vars[0] else {
            panic!("expected local assignment");
        };
        let decl = model.resolve_name(target.get_position()).unwrap();
        for phase in ["cold", "warm"] {
            assert_eq!(
                super::references_decl(&model, values[0].get_syntax_id(), &decl),
                references,
                "{rhs}: {phase}"
            );
        }
    }
}

#[test]
fn loop_recursive_call_rhs_does_not_claim_only_the_entry_type() {
    use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaNameExpr};
    for rhs_first in [false, true] {
        let mut ws = crate::VirtualWorkspace::new();
        let source = "---@param previous integer|string|nil\n---@return string?\nlocal function step(previous) return nil end\nlocal value=1; while flag do local observed=value; value=step(value) end";
        let file = ws.def(source);
        let expected = ws.ty("integer|string|nil");
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        if rhs_first {
            let assignment = chunk.descendants::<LuaAssignStat>().next().unwrap();
            let rhs = assignment.get_var_and_expr_list().1.remove(0);
            let _ = model.type_of_expr(rhs.get_syntax_id());
        }
        let offset = source.find("observed=value").unwrap() + "observed=".len();
        let read = chunk
            .descendants::<LuaNameExpr>()
            .find(|name| u32::from(name.get_position()) as usize == offset)
            .unwrap();
        for phase in ["cold", "warm"] {
            let actual = model.type_of_expr_at(read.get_syntax_id(), read.get_position());
            // Recursive call inference can remain unknown. Either keep that
            // uncertainty or the complete nullable return union; never claim
            // only the integer entry while dropping the possible later write.
            let uncertain = LuaType::from_vec(vec![LuaType::Integer, LuaType::Unknown]);
            assert!(
                actual == expected || actual == uncertain,
                "rhs_first={rhs_first}: {phase}: {actual:?}"
            );
        }
    }
}

#[test]
fn loop_carried_assignments_expand_only_bare_final_varargs() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for (assignment, expected) in [
        ("first, value = ...", "integer|string"),
        ("first, second, value = 0, ...", "integer|string"),
        ("first, value = (...) ", "integer|nil"),
    ] {
        let mut ws = crate::VirtualWorkspace::new();
        let source = format!(
            "---@param ... string\nlocal function run(...) local value=1; local first,second; while flag do local observed=value; {assignment} end end"
        );
        let file = ws.def(&source);
        let expected = ws.ty(expected);
        let model = ws.analysis.semantic_model(file);
        let offset = source.find("observed=value").unwrap() + "observed=".len();
        let read = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .find(|name| u32::from(name.get_position()) as usize == offset)
            .unwrap();
        for phase in ["cold", "warm"] {
            assert_eq!(
                model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
                expected,
                "{assignment}: {phase}"
            );
        }
    }
}

#[test]
fn loop_entry_keeps_unassigned_guards_and_constant_true_diagnostics() {
    let header =
        "---@class Item\n---@field name string\n---@param item Item?\nlocal function f(item)\n";
    for body in [
        "while item do print(item.name); break end",
        "while true do if item then print(item.name) end; break end",
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        assert!(
            ws.has_no_diagnostic(
                crate::DiagnosticCode::NeedCheckNil,
                &format!("{header}{body} end")
            ),
            "{body}"
        );
    }
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_diagnostic(
        crate::DiagnosticCode::NeedCheckNil,
        &format!("{header}while true do print(item.name); break end end")
    ));
    assert!(ws.has_diagnostic(
        crate::DiagnosticCode::UnnecessaryIf,
        "local count=0; while true do if count == 0 then print(count) end; break; count=1 end"
    ));
}

#[test]
fn loop_rhs_dependency_is_lazy_for_assignments_before_a_loop() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let source = "local count=0; count=count+1; local observed=count; while flag do break end";
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    let offset = source.find("observed=count").unwrap() + "observed=".len();
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| u32::from(name.get_position()) as usize == offset)
        .unwrap();
    for _ in 0..2 {
        model.type_of_expr_at(read.get_syntax_id(), read.get_position());
    }
    assert!(model.cache.borrow().loop_rhs_dependencies.is_empty());
}

#[test]
fn loop_structural_recurrence_does_not_claim_a_complete_shallow_union() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for assignment in ["value = value.next", "value = {next = value}"] {
        let mut ws = crate::VirtualWorkspace::new();
        let source = format!(
            "local value={{next={{next={{}}}}}}; while flag do local observed=value; {assignment} end"
        );
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let offset = source.find("observed=value").unwrap() + "observed=".len();
        let read = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .find(|name| u32::from(name.get_position()) as usize == offset)
            .unwrap();
        for phase in ["cold", "warm"] {
            let typ = model.type_of_expr_at(read.get_syntax_id(), read.get_position());
            let uncertain = match &typ {
                LuaType::Unknown | LuaType::Any => true,
                LuaType::Union(union) => union
                    .into_vec()
                    .iter()
                    .any(|part| matches!(part, LuaType::Unknown | LuaType::Any)),
                _ => false,
            };
            assert!(uncertain, "{assignment}: {phase}: {typ:?}");
        }
    }
}

#[test]
fn loop_conditional_fractional_update_widens_to_number() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};

    let mut ws = crate::VirtualWorkspace::new();
    // Read at loop entry, before the conditional write from this iteration.
    let source = "local index=0; while flag do\n---@type integer\nlocal result=index\nif other then index=index+0.5 end end";
    ws.analysis
        .diagnostic
        .enable_only(crate::DiagnosticCode::AssignTypeMismatch);
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    let result_decl_id = model
        .decl_by_offset((source.find("result=index").unwrap() as u32).into())
        .unwrap();
    let declaration = model
        .file_facts()
        .unwrap()
        .decl_by_id(&result_decl_id)
        .unwrap();
    assert!(
        declaration.doc_type_syntax.is_some(),
        "the result must own the integer annotation"
    );
    // Locate the index expression in the annotated result assignment.
    let read_offset = source.find("result=index").unwrap() + "result=".len();
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| u32::from(name.get_position()) as usize == read_offset)
        .unwrap();
    let read_type = model.type_of_expr_at(read.get_syntax_id(), read.get_position());
    let alternatives = match read_type {
        LuaType::Union(union) => union.into_vec(),
        other => vec![other],
    };
    assert!(alternatives.contains(&LuaType::Number), "{alternatives:?}");
    assert!(
        alternatives.iter().all(|ty| matches!(
            ty,
            LuaType::Integer | LuaType::IntegerConst(_) | LuaType::Number
        )),
        "{alternatives:?}"
    );
    // Match the diagnostic to that assignment, not the later fractional write.
    let result_line = source[..read_offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u32;
    let diagnostics = ws
        .analysis
        .diagnose_file(file, tokio_util::sync::CancellationToken::new())
        .unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == result_line
                && diagnostic.code
                    == Some(lsp_types::NumberOrString::String(
                        crate::DiagnosticCode::AssignTypeMismatch
                            .get_name()
                            .to_string()
                    ))),
        "the fractional result must be rejected: {diagnostics:?}"
    );
}

#[test]
fn statically_unentered_loop_preserves_the_preceding_value() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("local value=0; while false do value='text' end; local observed=value");
    let model = ws.analysis.semantic_model(file);
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .filter(|name| name.get_name_text().as_deref() == Some("value"))
        .last()
        .unwrap();
    assert_eq!(
        model.type_of_expr_at(read.get_syntax_id(), read.get_position()),
        LuaType::IntegerConst(0)
    );
}

#[test]
fn loop_structural_identity_and_independent_writes_keep_known_shapes() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    for assignment in ["value = value", "value = (value)", "value = {next={}}"] {
        let mut ws = crate::VirtualWorkspace::new();
        let source = format!(
            "local value={{next={{}}}}; while flag do local observed=value; {assignment} end"
        );
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let offset = source.find("observed=value").unwrap() + "observed=".len();
        let read = model
            .chunk()
            .unwrap()
            .descendants::<LuaNameExpr>()
            .find(|name| u32::from(name.get_position()) as usize == offset)
            .unwrap();
        let typ = model.type_of_expr_at(read.get_syntax_id(), read.get_position());
        let shapes = match typ {
            LuaType::Union(union) => union.into_vec(),
            other => vec![other],
        };
        assert!(
            shapes
                .iter()
                .all(|shape| matches!(shape, LuaType::TableConst(_))),
            "{assignment}: {shapes:?}"
        );
    }
}

#[test]
fn unrelated_function_loop_keeps_a_straight_line_read_on_the_fast_path() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let source = "local function unrelated() while flag do break end end;\nlocal function plain() local value=1; local observed=value; value=2 end";
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    let offset = source.find("observed=value").unwrap() + "observed=".len();
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| u32::from(name.get_position()) as usize == offset)
        .unwrap();
    let decl = model.resolve_name(read.get_position()).unwrap();
    super::super::flow_metrics::reset();
    assert_eq!(
        super::super::type_of_decl_at(&model, &decl, read.get_position()),
        LuaType::IntegerConst(1)
    );
    assert!(super::super::flow_metrics::fast_decl_hits() > 0);
}

#[test]
fn captured_assigned_parameters_keep_closure_entry_flow() {
    use emmylua_parser::{LuaAstNode, LuaNameExpr};
    let mut ws = crate::VirtualWorkspace::new();
    let source = "local function unrelated() while flag do break end end;\n---@param value integer\nlocal function outer(value) local function read() local observed=value end; value=1 end";
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    let offset = source.find("observed=value").unwrap() + "observed=".len();
    let read = model
        .chunk()
        .unwrap()
        .descendants::<LuaNameExpr>()
        .find(|name| u32::from(name.get_position()) as usize == offset)
        .unwrap();
    let decl = model.resolve_name(read.get_position()).unwrap();
    for position in [read.get_position(), read.get_range().end()] {
        super::super::flow_metrics::reset();
        assert_eq!(
            super::super::type_of_decl_at(&model, &decl, position),
            LuaType::Integer
        );
        assert_eq!(super::super::flow_metrics::fast_decl_hits(), 0);
    }
}
