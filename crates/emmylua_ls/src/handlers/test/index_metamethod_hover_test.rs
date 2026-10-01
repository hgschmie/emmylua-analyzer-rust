#![cfg(test)]

use emmylua_code_analysis::FileId;
use googletest::prelude::*;
use lsp_types::{HoverContents, Position};

use crate::handlers::{hover::hover, test_lib::ProviderVirtualWorkspace};

const PROVIDER: &str = r#"local Is = {}
local M = {}
--- Checks whether the value is callable.
---@param value any The value to inspect.
---@return boolean result Whether the value can be called.
function M.Callable(value) return type(value) == 'function' end
M.Function = M.Callable
--- Returns the value if it is a table.
-- @tparam mixed value The table candidate.
-- @treturn mixed
function M.Table(value) return type(value) == 'table' and value end
setmetatable(Is, {
    __index = function(_, key)
        return M[key] and function(actual) return M[key](actual) end or nil
    end,
})
return Is
"#;

fn hover_text(ws: &ProviderVirtualWorkspace, file: FileId, line: u32, column: u32) -> String {
    match hover(&ws.analysis, file, Position::new(line, column)) {
        Some(result) => match result.contents {
            HoverContents::Markup(markup) => markup.value,
            contents => panic!("unexpected hover contents: {contents:?}"),
        },
        None => String::new(),
    }
}

#[gtest]
fn index_metamethod_hover_includes_backing_documentation() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", PROVIDER);
    let file = ws.def_file("consumer.lua", "local Is = require('predicates')\nIs.Function(function() end)\nlocal predicate = Is.Function\nIs['Function'](nil)\nIs.Table({})");
    for (line, column) in [(1, 5), (2, 23), (3, 6)] {
        let text = hover_text(&ws, file, line, column);
        assert_that!(
            &text,
            contains_substring("Checks whether the value is callable.")
        );
        assert_that!(
            &text,
            contains_substring("@*param* `value` — The value to inspect.")
        );
        assert_that!(
            &text,
            contains_substring("Whether the value can be called.")
        );
        assert_that!(&text, contains_substring("M.Callable"));
        // The displayed type still describes the wrapper, not its callee.
        assert_that!(&text, contains_substring("fun(actual)"));
    }
    let table = hover_text(&ws, file, 4, 5);
    assert_that!(
        &table,
        contains_substring("Returns the value if it is a table.")
    );
    assert_that!(
        &table,
        contains_substring("@tparam mixed value The table candidate.")
    );
}

#[gtest]
fn index_metamethod_hover_keeps_explicit_member_and_alias_docs() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "predicates.lua",
        &PROVIDER.replace(
            "M.Function =",
            "--- Alias-specific documentation.\nM.Function =",
        ),
    );
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Function(nil)",
    );
    let text = hover_text(&ws, file, 1, 5);
    assert_that!(&text, contains_substring("Alias-specific documentation."));
    assert_that!(
        &text,
        not(contains_substring("Checks whether the value is callable."))
    );

    ws.def_file("predicates.lua", &PROVIDER.replace("local M = {}", "--- Public member documentation.\nfunction Is.Function(public_value) return public_value end\nlocal M = {}"));
    let text = hover_text(&ws, file, 1, 5);
    assert_that!(&text, contains_substring("Public member documentation."));
    assert_that!(
        &text,
        not(contains_substring("Checks whether the value is callable."))
    );
}

#[gtest]
fn index_metamethod_hover_documents_direct_lookups() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "predicates.lua",
        &PROVIDER.replace(
            "return M[key] and function(actual) return M[key](actual) end or nil",
            "return M[key]",
        ),
    );
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Function(nil)",
    );
    let text = hover_text(&ws, file, 1, 5);
    assert_that!(
        &text,
        contains_substring("Checks whether the value is callable.")
    );
    assert_that!(&text, contains_substring("The value to inspect."));
    assert_that!(
        &text,
        contains_substring("Whether the value can be called.")
    );
}

#[gtest]
fn index_metamethod_hover_documents_nonfunction_members() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", "local proxy = {}\nlocal M = {\n--- The answer.\nanswer = 42,\n--- First entry.\n[1] = 12,\n}\nsetmetatable(proxy, {__index = function(_, key) return M[key] end})\nreturn proxy");
    let file = ws.def_file(
        "consumer.lua",
        "local p = require('predicates')\nprint(p.answer)\nprint(p[1])",
    );
    let text = hover_text(&ws, file, 1, 10);
    assert_that!(&text, contains_substring("The answer."));
    assert_that!(&text, contains_substring("42"));
    let text = hover_text(&ws, file, 2, 8);
    assert_that!(&text, contains_substring("First entry."));
    assert_that!(&text, contains_substring("12"));
}

#[gtest]
fn index_metamethod_hover_updates_without_reopening_consumer() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", PROVIDER);
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Function(nil)",
    );
    assert_that!(
        hover_text(&ws, file, 1, 5),
        contains_substring("The value to inspect.")
    );
    ws.def_file(
        "predicates.lua",
        &PROVIDER.replace("The value to inspect.", "Updated parameter documentation."),
    );
    let text = hover_text(&ws, file, 1, 5);
    assert_that!(
        &text,
        contains_substring("Updated parameter documentation.")
    );
    assert_that!(&text, not(contains_substring("The value to inspect.")));
    ws.def_file(
        "predicates.lua",
        &PROVIDER.replace("M.Function =", "M.Renamed ="),
    );
    assert_that!(
        hover_text(&ws, file, 1, 5),
        not(contains_substring("Checks whether"))
    );
    ws.def_file("predicates.lua", PROVIDER);
    assert_that!(
        hover_text(&ws, file, 1, 5),
        contains_substring("The value to inspect.")
    );
}

#[gtest]
fn index_metamethod_hover_does_not_borrow_unrelated_docs() {
    for replacement in [
        "return M[key] and function(actual) return true end or nil",
        "return M[key] and function(key) return M[key](nil) end or nil",
        "if M[key] then return M[key] end; return nil",
    ] {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "predicates.lua",
            &PROVIDER.replace(
                "return M[key] and function(actual) return M[key](actual) end or nil",
                replacement,
            ),
        );
        let file = ws.def_file("consumer.lua", "local Is = require('predicates')\nIs.Function(nil)\nIs.Missing(nil)\nlocal key = ...\nIs[key](nil)");
        for (line, column) in [(1, 5), (2, 5), (4, 4)] {
            assert_that!(
                hover_text(&ws, file, line, column),
                not(contains_substring("Checks whether")),
                "body: {replacement}"
            );
        }
    }
}

#[gtest]
fn index_metamethod_hover_preserves_wrapper_signature() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "predicates.lua",
        &PROVIDER.replace(
            "function(actual) return M[key](actual)",
            "function(first, second) return M[key](nil)",
        ),
    );
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Function(1, 2)",
    );
    let text = hover_text(&ws, file, 1, 5);
    assert_that!(&text, contains_substring("fun(first, second)"));
    assert_that!(
        &text,
        contains_substring("Checks whether the value is callable.")
    );
    assert_that!(&text, contains_substring("Documentation from"));
}

#[gtest]
fn index_metamethod_hover_keeps_wrapper_documentation() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", &PROVIDER.replace(
        "function(actual) return M[key](actual)",
        "\n--- Wrapper-specific documentation.\n---@param actual number Wrapper input.\nfunction(actual) return M[key](actual)",
    ));
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Function(1)",
    );
    let text = hover_text(&ws, file, 1, 5);
    assert_that!(&text, contains_substring("Wrapper-specific documentation."));
    assert_that!(&text, contains_substring("Wrapper input."));
    assert_that!(&text, not(contains_substring("The value to inspect.")));
    assert_that!(&text, not(contains_substring("Documentation from")));
}
