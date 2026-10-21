#![cfg(test)]

use emmylua_code_analysis::FileId;
use googletest::prelude::*;
use lsp_types::{GotoDefinitionResponse, Location, Position};

use crate::handlers::{definition::definition, test_lib::ProviderVirtualWorkspace};

const PREDICATES: &str = "local Is = {}\nlocal M = {}\nfunction M.Table(value) return value end\nfunction M.Callable(value) return type(value) == 'function' end\nM.Function = M.Callable\nsetmetatable(Is, {\n    __index = function(_, key)\n        return M[key] and function(value) return M[key](value) end or nil\n    end,\n})\nreturn Is\n";

fn locations(ws: &ProviderVirtualWorkspace, file: FileId, line: u32, column: u32) -> Vec<Location> {
    match definition(&ws.analysis, file, Position::new(line, column)) {
        Some(GotoDefinitionResponse::Scalar(location)) => vec![location],
        Some(GotoDefinitionResponse::Array(locations)) => locations,
        None => vec![],
        result => panic!("unexpected definition response: {result:?}"),
    }
}

fn target(ws: &ProviderVirtualWorkspace, line: u32, start: u32, end: u32) -> Vec<Location> {
    vec![Location::new(
        ws.virtual_url_generator.new_uri("predicates.lua"),
        lsp_types::Range::new(Position::new(line, start), Position::new(line, end)),
    )]
}

#[gtest]
fn index_metamethod_definition_resolves_wrapper_and_alias() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", PREDICATES);
    let file = ws.def_file("consumer.lua", "local Is = require('predicates')\nIs.Table({})\nIs.Function(function() end)\nlocal predicate = Is.Table\nIs['Table']({})");
    assert_that!(locations(&ws, file, 1, 5), eq(&target(&ws, 2, 11, 16)));
    assert_that!(locations(&ws, file, 2, 5), eq(&target(&ws, 3, 11, 19)));
    assert_that!(locations(&ws, file, 3, 23), eq(&target(&ws, 2, 11, 16)));
    assert_that!(locations(&ws, file, 4, 6), eq(&target(&ws, 2, 11, 16)));
}

#[gtest]
fn index_metamethod_definition_updates_existing_consumer() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", PREDICATES);
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Table({})\nIs.TableX({})",
    );
    assert_that!(locations(&ws, file, 1, 5), eq(&target(&ws, 2, 11, 16)));
    assert_that!(locations(&ws, file, 2, 5), is_empty());
    ws.def_file(
        "predicates.lua",
        &PREDICATES
            .replace("local M = {}", "\n\nlocal M = {}")
            .replace("M.Table", "M.TableX"),
    );
    assert_that!(locations(&ws, file, 1, 5), is_empty());
    assert_that!(locations(&ws, file, 2, 5), eq(&target(&ws, 4, 11, 17)));
    ws.def_file("predicates.lua", PREDICATES);
    assert_that!(locations(&ws, file, 1, 5), eq(&target(&ws, 2, 11, 16)));
}

#[gtest]
fn index_metamethod_definition_direct_lookup_and_integer_key() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file("predicates.lua", "local M = {\n    answer = 42,\n    [1] = 'first',\n}\nlocal proxy = {}\nsetmetatable(proxy, { __index = function(_, key) return M[key] end })\nreturn proxy");
    let file = ws.def_file(
        "consumer.lua",
        "local v = require('predicates')\nprint(v.answer)\nprint(v[1])\nprint(v[2])",
    );
    assert_that!(locations(&ws, file, 1, 10), eq(&target(&ws, 1, 4, 10)));
    assert_that!(locations(&ws, file, 2, 8), eq(&target(&ws, 2, 5, 6)));
    assert_that!(locations(&ws, file, 3, 8), is_empty());
}

#[gtest]
fn index_metamethod_definition_preserves_normal_members() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "predicates.lua",
        &PREDICATES.replace(
            "local Is = {}",
            "local Is = {}\nfunction Is.Table(value) return value end",
        ),
    );
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Table({})",
    );
    assert_that!(locations(&ws, file, 1, 5), eq(&target(&ws, 1, 12, 17)));
}

#[gtest]
fn index_metamethod_definition_does_not_guess() {
    for body in [
        "return M[key] and function(value) return M[key](value) end or nil",
        "return function() return 42 end",
        "return M[key] and function() return 42 end or nil",
        "return M[key] or function() return 42 end",
        "if M[key] then return M[key] end; return function() end",
        "return M[key] and function(key) return M[key]() end or nil",
    ] {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        let source = PREDICATES.replace(
            "return M[key] and function(value) return M[key](value) end or nil",
            body,
        );
        ws.def_file("predicates.lua", &source);
        let file = ws.def_file(
            "consumer.lua",
            "local Is = require('predicates')\nIs.Missing()\nlocal key = ...\nIs[key]()",
        );
        assert_that!(locations(&ws, file, 1, 5), is_empty(), "body: {body}");
        // The variable used as a computed key keeps its ordinary declaration.
        let key_target = locations(&ws, file, 3, 4);
        assert_that!(key_target.len(), eq(1));
        assert_that!(key_target[0].range.start.line, eq(2));
        if body.starts_with("return function")
            || body.contains("and function() return 42")
            || body.contains("function(key)")
        {
            let valid = ws.def_file(
                "known.lua",
                "local Is = require('predicates')\nIs.Table({})",
            );
            assert_that!(locations(&ws, valid, 1, 5), is_empty(), "body: {body}");
        }
    }
}

#[gtest]
fn index_metamethod_definition_follows_the_selected_value() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "predicates.lua",
        &PREDICATES.replace(
            "return M[key] and function(value) return M[key](value) end or nil",
            "return M[key] or M.Callable",
        ),
    );
    let file = ws.def_file(
        "consumer.lua",
        "local Is = require('predicates')\nIs.Table({})\nIs.Missing({})",
    );
    assert_that!(locations(&ws, file, 1, 5), eq(&target(&ws, 2, 11, 16)));
    assert_that!(locations(&ws, file, 2, 5), eq(&target(&ws, 3, 11, 19)));
    // A wrapper forwarding to a different function should follow its callee,
    // not the key whose presence happened to select that wrapper.
    ws.def_file(
        "predicates.lua",
        &PREDICATES.replace("return M[key](value)", "return M.Callable(value)"),
    );
    assert_that!(locations(&ws, file, 1, 5), eq(&target(&ws, 3, 11, 19)));
    assert_that!(locations(&ws, file, 2, 5), is_empty());
}

#[gtest]
fn index_metamethod_definition_keeps_ambiguous_and_open_tables_unresolved() {
    for extra in [
        "M.Table = function() end",
        "function M.Table(value) return value end",
        "if ... then M.Table = function() end end",
        "local key = ...; M[key] = function() end",
        "populate(M)",
    ] {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "predicates.lua",
            &PREDICATES.replace("setmetatable(Is,", &format!("{extra}\nsetmetatable(Is,")),
        );
        let file = ws.def_file(
            "consumer.lua",
            "local Is = require('predicates')\nIs.Table({})",
        );
        assert_that!(locations(&ws, file, 1, 5), is_empty(), "extra: {extra}");
    }
}

#[gtest]
fn index_metamethod_definition_follows_parenthesized_wrapper_call() {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "predicates.lua",
        &PREDICATES.replace("return M[key](value)", "return ((M[key](value)))"),
    );
    let file = ws.def_file(
        "consumer.lua",
        "local Is=require('predicates')\nIs.Table({})",
    );
    assert_that!(locations(&ws, file, 1, 5), eq(&target(&ws, 2, 11, 16)));
}
