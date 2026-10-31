use crate::{DiagnosticCode, LuaType, VirtualWorkspace};
use googletest::prelude::*;

#[gtest]
fn annotated_runtime_member_dictionary_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type table<string,string>\nC.names = {}");
    let expected = ws.ty("table<string,string>");
    expect_that!(ws.expr_ty("C.names"), eq(&expected));
}

#[gtest]
fn annotated_runtime_member_array_chain() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type string[]\nC.names = {}");
    let expected = ws.ty("string?");
    expect_that!(ws.expr_ty("C.names[1]"), eq(&expected));
}

#[gtest]
fn annotated_runtime_member_cross_file_class() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file("config.lua", "---@class Config\n---@field name string\nlocal C = {}\n---@type Config\nC.config = {}\nreturn C");
    expect_that!(
        ws.expr_ty("require('config').config.name"),
        eq(&LuaType::String)
    );
}

#[gtest]
fn annotated_runtime_member_does_not_hide_invalid_initializer() {
    let mut ws = VirtualWorkspace::new();
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            "local C = {}\n---@type string\nC.name = 42"
        ),
        eq(false)
    );
}

#[gtest]
fn annotated_runtime_member_nullable_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type string?\nC.name = nil");
    let expected = ws.ty("string?");
    expect_that!(ws.expr_ty("C.name"), eq(&expected));
}

#[gtest]
fn annotated_runtime_member_object_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type {name: string}\nC.config = {}");
    expect_that!(ws.expr_ty("C.config.name"), eq(&LuaType::String));
}

#[gtest]
fn annotated_runtime_member_function_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type fun():string\nC.name = function() return 'one' end");
    expect_that!(ws.expr_ty("C.name()"), eq(&LuaType::String));
}

#[gtest]
fn annotated_runtime_member_named_string_field_is_not_an_index() {
    let mut ws = VirtualWorkspace::new();
    ws.def("---@class Config\n---@field string string\nC = {}\n---@type Config\nC.config = {}");
    expect_that!(
        ws.has_no_diagnostic(DiagnosticCode::InjectField, "C.config.extra = true"),
        eq(false)
    );
}

#[gtest]
fn annotated_runtime_member_index_contracts_validate_keys_and_named_class_values() {
    for contract in ["NumericRecord", "{[integer]: string, named: boolean}"] {
        let mut cases = vec![
            ("C.config[1] = 'ok'", vec![]),
            ("C.config.named = false", vec![]),
            ("C.config['1'] = 'ok'", vec![DiagnosticCode::InjectField]),
            ("C.config.extra = true", vec![DiagnosticCode::InjectField]),
        ];
        if contract == "NumericRecord" {
            cases.push((
                "C.config.named = 'wrong'",
                vec![DiagnosticCode::AssignTypeMismatch],
            ));
        }
        for (operation, expected) in cases {
            let source = format!(
                r#"
                ---@class NumericRecord
                ---@field [integer] string
                ---@field named boolean
                local Record = {{}}
                local C = {{}}
                ---@type {contract}
                C.config = {{ named = true }}
                {operation}
                return C
            "#
            );
            for code in [
                DiagnosticCode::AssignTypeMismatch,
                DiagnosticCode::InjectField,
            ] {
                let mut ws = VirtualWorkspace::new();
                expect_that!(
                    ws.has_no_diagnostic(code, &source),
                    eq(!expected.contains(&code)),
                    "{contract}, {operation}: {code:?}"
                );
            }
        }
    }
}

#[gtest]
fn annotated_runtime_member_shell_and_rich_queries_agree() {
    use crate::LuaMemberKey;
    let mut ws = VirtualWorkspace::new();
    let file = ws.def("local C = {}\n---@type table<string,string>\nC.names = {}\nreturn C");
    let foreign_file = ws.def("local unrelated = true");
    let expected = ws.ty("table<string,string>");
    for model_file in [file, foreign_file] {
        for rich_first in [false, true] {
            let model = ws.analysis.semantic_model(model_file);
            let facts = model.file_facts_of(file).unwrap();
            let member = facts
                .members
                .iter()
                .find(|member| member.key == LuaMemberKey::from("names"))
                .unwrap();
            assert!(member.doc_type_syntax.is_some() && member.value_syntax.is_some());
            assert_ne!(member.doc_type_syntax, member.value_syntax);
            if rich_first {
                expect_that!(
                    model.type_of_member(&member.id),
                    eq(&Some(expected.clone()))
                );
            }
            let shell = model
                .analysis()
                .member_type(file, member.id.clone())
                .unwrap();
            let projected = model.analysis().type_shell_lua(file, &shell);
            expect_that!(
                projected,
                eq(&expected),
                "foreign={}, rich_first={rich_first}",
                model_file != file
            );
            expect_that!(
                model.type_of_member(&member.id),
                eq(&Some(expected.clone()))
            );
        }
    }
}

#[gtest]
fn annotated_runtime_member_inline_annotation_preserves_initializer_checks() {
    for (value, valid) in [("'ready'", true), ("42", false)] {
        let source = format!("C = {{\n---@type string\nname = {value}\n}}");
        let mut ws = VirtualWorkspace::new();
        ws.def(&source);
        expect_that!(ws.expr_ty("C.name"), eq(&LuaType::String), "{value}");
        let mut ws = VirtualWorkspace::new();
        expect_that!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
            eq(valid),
            "{value}"
        );
    }
}
