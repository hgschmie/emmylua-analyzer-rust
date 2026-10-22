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
