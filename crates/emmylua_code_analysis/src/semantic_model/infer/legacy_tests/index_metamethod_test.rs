#![cfg(test)]

use googletest::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::{DiagnosticCode, FileId, VirtualWorkspace};

const PREDICATES: &str = r#"
local Is = {}
local M = {}
function M.Table(value) return type(value) == 'table' and value end
function M.Function(value) return type(value) == 'function' end
setmetatable(Is, {
    __index = function(_, key)
        return M[key] and function(value) return M[key](value) end or nil
    end,
})
return Is
"#;

fn undefined_fields(ws: &VirtualWorkspace, file: FileId) -> Vec<u32> {
    ws.analysis
        .diagnose_file(file, CancellationToken::new())
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.code == Some(lsp_types::NumberOrString::String("undefined-field".into())))
        .map(|d| d.range.start.line)
        .collect()
}

#[gtest]
fn index_function_distinguishes_existing_and_missing_keys() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", PREDICATES);
    let file = ws.def_file("consumer.lua", "local Is = require('predicates')\nIs.Table({})\nIs.Function(function() end)\nIs.Foobar({})");
    assert_that!(undefined_fields(&ws, file), eq(&vec![3]));
    let valid = ws.expr_ty("require('predicates').Table");
    assert_that!(valid.is_function(), eq(true));
}

#[gtest]
fn index_function_rechecks_existing_consumer_after_edits() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", PREDICATES);
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Table({})\nIs.TableX({})\nIs.Foobar({})",
    );
    assert_that!(undefined_fields(&ws, file), eq(&vec![2, 3]));
    ws.def_file("predicates.lua", &PREDICATES.replace("M.Table", "M.TableX"));
    assert_that!(undefined_fields(&ws, file), eq(&vec![1, 3]));
    ws.def_file("predicates.lua", PREDICATES);
    assert_that!(undefined_fields(&ws, file), eq(&vec![2, 3]));
}

#[gtest]
fn index_function_direct_lookup_and_literal_keys() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "values.lua",
        r#"
        local values = { answer = 42, [1] = 'first', disabled = false }
        local proxy = {}
        setmetatable(proxy, { __index = function(_, key) return (values[key]) end })
        return proxy
    "#,
    );
    let answer = ws.expr_ty("require('values')['answer']");
    assert_that!(ws.humanize_type(answer), eq("42"));
    let first = ws.expr_ty("require('values')[1]");
    assert_that!(ws.humanize_type(first), eq("\"first\""));
    let file = ws.def_file(
        "consumer.lua",
        "local v = require('values')\nprint(v.disabled)\nprint(v.missing)\nprint(v[2])",
    );
    assert_that!(undefined_fields(&ws, file), eq(&vec![2, 3]));
}

#[gtest]
fn index_function_dynamic_fallbacks_remain_valid() {
    for body in [
        "return function(value) return value end",
        "return M[key] or function(value) return value end",
        "return M[key] and function(value) return value end or function() end",
        "if M[key] then return M[key] end; return function() end",
        "key = 'Table'; return M[key]",
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "dynamic.lua",
            &format!(
                r#"
            local M = {{ Table = function(value) return value end }}
            local proxy = {{}}
            setmetatable(proxy, {{ __index = function(_, key) {body} end }})
            return proxy
        "#
            ),
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::UndefinedField,
                "require('dynamic').Foobar({})"
            ),
            eq(true)
        );
    }
}

#[gtest]
fn index_function_open_and_reassigned_tables_remain_permissive() {
    for setup in [
        "---@type table<string, fun()>\nlocal M = {}",
        "local M = {}; local key = ...; M[key] = function() end",
        "local M = {}; M = get_predicates()",
        "local M = {}; populate(M)",
        "local M = {}; local alias = M; alias.Foobar = function() end",
        "local original = {}; local M = original; populate(original)",
        "local M = {}; M:populate()",
        "local key = ...; local M = { [key] = function() end }",
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "dynamic.lua",
            &format!(
                r#"
            {setup}
            local proxy = {{}}
            setmetatable(proxy, {{ __index = function(_, key)
                return M[key] and function(value) return M[key](value) end or nil
            end }})
            return proxy
        "#
            ),
        );
        assert_that!(
            ws.has_no_diagnostic(
                DiagnosticCode::UndefinedField,
                "require('dynamic').Foobar({})"
            ),
            eq(true),
            "setup: {setup}"
        );
    }
}

#[gtest]
fn index_function_does_not_assume_nested_tables_are_closed() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "nested.lua",
        r#"
        local M = { predicates = {} }
        populate(M.predicates)
        local proxy = {}
        setmetatable(proxy, { __index = function(_, key)
            return M.predicates[key] and function() end or nil
        end })
        return proxy
    "#,
    );
    assert_that!(
        ws.has_no_diagnostic(DiagnosticCode::UndefinedField, "require('nested').Foobar()"),
        eq(true)
    );
}

#[gtest]
fn index_function_preserves_declared_fields_and_dynamic_access() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "predicates.lua",
        &PREDICATES.replace("local Is = {}", "local Is = { Foobar = function() end }"),
    );
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Foobar()\nlocal key = ...\nIs[key]()\nIs.Missing()",
    );
    assert_that!(undefined_fields(&ws, file), eq(&vec![4]));
}

#[gtest]
fn index_function_specialization_is_bounded() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    // A very long supported expression must fall back without growing the
    // native inference stack in proportion to source size.
    let expr = format!("{}key", "nil or ".repeat(1000));
    ws.def_file("deep.lua", &format!("local proxy = {{}}; setmetatable(proxy, {{ __index = function(_, key) return {expr} end }}); return proxy"));
    assert_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::UndefinedField,
            "print(require('deep').Foobar)"
        ),
        eq(true)
    );
}

#[gtest]
fn index_function_respects_key_parameter_type() {
    for annotation in ["'answer'", "'answer'|integer"] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "typed.lua",
            &r#"
        local values = { answer = 42 }
        local proxy = {}
        setmetatable(proxy, {
            ---@param key 'answer'
            __index = function(_, key) return values[key] end,
        })
        return proxy
    "#
            .replace("@param key 'answer'", &format!("@param key {annotation}")),
        );
        let file = ws.def_file(
            "consumer.lua",
            "local v = require('typed')\nprint(v.answer)\nprint(v.wrong)",
        );
        assert_that!(undefined_fields(&ws, file), eq(&vec![2]));
    }
}

#[gtest]
fn index_function_respects_explicit_return_annotations() {
    for annotation in ["number", "any"] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "annotated.lua",
            &r#"
        local M = {}
        local proxy = {}
        setmetatable(proxy, {
            ---@param key string
            ---@return number
            __index = function(_, key) return M[key] end,
        })
        return proxy
    "#
            .replace("return number", &format!("return {annotation}")),
        );
        let typ = ws.expr_ty("require('annotated').Foobar");
        assert_that!(ws.humanize_type(typ), eq(annotation));
    }
}
