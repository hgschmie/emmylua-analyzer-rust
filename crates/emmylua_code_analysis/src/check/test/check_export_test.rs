//! Tests for the check_export checker (mirrors the test_export family from the old `diagnostic/test/inject_field_test.rs`).

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use lsp_types::Uri;

use crate::DiagnosticCode;
use crate::{Emmyrc, LuaType, SemanticDatabase, SemanticModel, VirtualWorkspace};

use super::{Diagnostic, check_source, count_by_code};
use crate::check::{CheckConfig, check_file};

fn check_export(def_source: &str, use_source: &str) -> Vec<Diagnostic> {
    let emmyrc = Arc::new(Emmyrc::default());
    let mut db = SemanticDatabase::new();
    db.update_config(emmyrc.clone());

    let def_uri = Uri::from_str("file:///C:/ws/mod.lua").expect("def uri");
    db.set_file_content(&def_uri, Some(def_source.to_string()));
    let use_uri = Uri::from_str("file:///C:/ws/main.lua").expect("use uri");
    let use_file = db.set_file_content(&use_uri, Some(use_source.to_string()));
    db.update_main_root(PathBuf::from("C:/ws"));

    let model = SemanticModel::new(&db, use_file);
    let config = Arc::new(CheckConfig::new(&emmyrc));
    check_file(&model, config)
}

// 导出类表时应包含声明字段, 同时继续拒绝不存在的字段.
#[test]
fn test_export_declared_class_fields() {
    let source = r#"
        ---@class Shared
        ---@field value integer
        local M = {}
        return M
        "#;
    let mut ws = VirtualWorkspace::new();
    ws.def_file("mod.lua", source);
    assert_eq!(ws.expr_ty("require('mod').value"), LuaType::Integer);
    let diagnostics = check_export(
        source,
        r#"
        local shared = require 'mod'
        shared.value = 1
        return shared.value, shared.missing
        "#,
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::UndefinedField),
        1,
        "{diagnostics:#?}"
    );
    assert_eq!(
        count_by_code(&diagnostics, DiagnosticCode::InjectField),
        0,
        "{diagnostics:#?}"
    );
}

#[test]
fn imported_missing_fields_report_once_per_key() {
    for read in ["m.absent", "m['absent']"] {
        let diagnostics = check_export(
            "---@class Exported\nlocal M = {}\nreturn M",
            &format!("local m = require 'mod'\nreturn {read}"),
        );
        assert_eq!(
            count_by_code(&diagnostics, DiagnosticCode::UndefinedField),
            1,
            "{read}: {diagnostics:#?}"
        );
    }
}

/// `return { a = 1 }`: imported tables may only read/write fields on the export surface.
#[test]
fn test_export_table_literal() {
    let diags = check_export(
        "return { a = 1 }",
        r#"
        local a = require("mod")
        a.newField = 1
        "#,
    );
    assert_eq!(count_by_code(&diags, DiagnosticCode::InjectField), 1);

    let diags = check_export(
        "return { a = 1 }",
        r#"
        local a = require("mod")
        a.a = 2
        "#,
    );
    assert_eq!(count_by_code(&diags, DiagnosticCode::InjectField), 0);

    let diags = check_export(
        "return { a = 1 }",
        r#"
        local a = require("mod")
        local v = a.newField
        "#,
    );
    assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedField), 1);
}

/// `local export = {}; export.a = 1; return export`: runtime members are also part of the export surface.
#[test]
fn test_export_runtime_members() {
    let diags = check_export(
        r#"
        local export = {}
        export.a = 1
        return export
        "#,
        r#"
        local a = require("mod")
        a.a = 2
        "#,
    );
    assert_eq!(
        count_by_code(&diags, DiagnosticCode::InjectField),
        0,
        "diagnostics: {:?}",
        diags
    );

    let diags = check_export(
        r#"
        local export = {}
        export.a = 1
        return export
        "#,
        r#"
        local a = require("mod")
        a.newField = 1
        "#,
    );
    assert_eq!(count_by_code(&diags, DiagnosticCode::InjectField), 1);
}

/// Direct `require("mod").newField = 1`.
#[test]
fn test_export_direct_require() {
    let diags = check_export(
        "return { a = 1 }",
        r#"
        require("mod").newField = 1
        "#,
    );
    assert_eq!(count_by_code(&diags, DiagnosticCode::InjectField), 1);
}

/// Non-require local tables are unrestricted.
#[test]
fn test_local_table_not_limited() {
    let diags = check_source(
        r#"
        local t = {}
        t.newField = 1
        local v = t.another
        "#,
    );
    assert_eq!(count_by_code(&diags, DiagnosticCode::InjectField), 0);
    assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedField), 0);
}

#[test]
fn imported_class_exports_validate_missing_fields() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file(
        "framework.lua",
        r#"
        ---@class ExportSettings
        local Settings = {}
        function Settings:add_defaults(definitions) end

        ---@class ExportRoot
        ---@field settings ExportSettings
        local Root = {}
        function Root:init() end
        return Root
        "#,
    );
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UndefinedField,
        r#"
        local framework = require("framework")
        framework:init()
        framework.settings:add_defaults({})
        local settings = require("framework").settings
        "#,
    ));
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::InjectField,
        r#"
        local framework = require("framework")
        framework.settings = framework.settings
        "#,
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::UndefinedField,
        r#"local missing = require("framework").missing"#,
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::InjectField,
        r#"require("framework").newField = 1"#,
    ));
    for code in [DiagnosticCode::InjectField, DiagnosticCode::UndefinedField] {
        assert!(!ws.has_no_diagnostic(
            code,
            r#"
            local framework = require("framework")
            ---@type integer
            framework.consumerField = 1
            local value = framework.consumerField
            "#,
        ));
    }
}

#[test]
fn test_export_annotated_inherited_fields() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file(
        "types.lua",
        r#"
        ---@class ExportBase
        ---@field inherited integer
        ---@class ExportChild: ExportBase
        ---@field declared string
        "#,
    );
    ws.def_file(
        "annotated.lua",
        r#"
        ---@type ExportChild
        local M = {}
        return M
        "#,
    );
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UndefinedField,
        r#"
        local m = require("annotated")
        local a, b = m.inherited, m.declared
        "#,
    ));
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::InjectField,
        r#"
        local m = require("annotated")
        m.inherited = 1
        m.declared = "ok"
        "#,
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::UndefinedField,
        r#"local missing = require("annotated").missing"#,
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::InjectField,
        r#"require("annotated").newField = 1"#,
    ));
}

#[test]
fn test_export_annotation_preserves_runtime_surface() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file(
        "loose.lua",
        r#"
        ---@type any
        local M = { existing = 1 }
        return M
        "#,
    );
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UndefinedField,
        r#"local value = require("loose").existing"#,
    ));
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UndefinedField,
        r#"return require("loose").dynamic"#
    ));
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::InjectField,
        r#"require("loose").newField = true"#
    ));
}

#[test]
fn open_annotations_allow_new_export_fields() {
    for annotation in ["table", "unknown", "table<string, integer>"] {
        let module = format!("---@type {annotation}\nlocal M = {{}}\nreturn M");
        let result = check_export(&module, "local m = require 'mod'\nm.newField = 1");
        assert_eq!(
            count_by_code(&result, DiagnosticCode::InjectField),
            0,
            "{annotation}: {result:?}"
        );
    }
}

#[test]
fn factory_exports_use_the_declared_class() {
    let module = "---@class Exported\n---@field name string\nlocal Foo = {}\n---@return Exported\nlocal function make() return Foo end\nreturn make()";
    let result = check_export(module, "local m = require 'mod'\nreturn m.name, m.absent");
    assert_eq!(
        count_by_code(&result, DiagnosticCode::UndefinedField),
        1,
        "{result:?}"
    );
}
