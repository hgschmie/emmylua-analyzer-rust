use super::check_source;
use crate::DiagnosticCode;

const DECLARATION: &str = r#"
---@param values table<integer, string>
---@param mode string
---@overload fun(values: table<string, integer>, mode: integer)
local function accept(values, mode) return values, mode end
"#;

#[test]
fn composed_overload_accepts_each_complete_local_signature() {
    for call in ["accept({'a'}, 'a')", "accept({count = 1}, 1)"] {
        let diagnostics = check_source(&format!("{DECLARATION}{call}"));
        assert!(diagnostics.is_empty(), "{call}: {diagnostics:?}");
    }
}

#[test]
fn composed_overload_rejects_mixed_signatures_at_the_invalid_call() {
    for call in ["accept({count = 1}, 'a')", "accept({'a'}, 1)"] {
        let source = format!("{DECLARATION}{call}");
        let diagnostics = check_source(&source);
        assert_eq!(diagnostics.len(), 1, "{source}\n{diagnostics:?}");
        assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
        assert!(usize::from(diagnostics[0].range.start()) >= DECLARATION.len());
    }
}

#[test]
fn composed_overload_combined_error_is_on_third_call_only() {
    let valid = "accept({'a'}, 'a')\naccept({count = 1}, 1)\n";
    let source = format!("{DECLARATION}{valid}accept({{count = 1}}, 'a')");
    let diagnostics = check_source(&source);
    assert_eq!(diagnostics.len(), 1, "{source}\n{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
    assert!(usize::from(diagnostics[0].range.start()) >= DECLARATION.len() + valid.len());
}

#[test]
fn composed_overload_keeps_invalid_dictionary_values() {
    let diagnostics = check_source(&format!("{DECLARATION}accept({{count = 'bad'}}, 1)"));
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::AssignTypeMismatch)
    );
}

#[test]
fn composed_overload_keeps_declared_dictionary_assignment_checks() {
    let diagnostics = check_source(
        "---@type table<string, integer>\nlocal values = {count = 'bad'}\nreturn values",
    );
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::AssignTypeMismatch);
}

#[test]
fn composed_overload_respects_explicit_function_type() {
    let source = "---@type fun(values: table<integer, string>, mode: string)\n---@overload fun(values: table<string, integer>, mode: integer)\nlocal accept = function(values, mode) return values, mode end\naccept({count = 1}, 1)";
    let diagnostics = check_source(source);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn composed_overload_does_not_restore_reassigned_declaration_overloads() {
    let source = format!(
        "{DECLARATION}\n---@param values table<integer, string>\n---@param mode string\nlocal function replacement(values, mode) return values, mode end\naccept = replacement\naccept({{count = 1}}, 1)"
    );
    let diagnostics = check_source(&source);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn composed_overload_respects_cast_to_the_primary_signature() {
    let source = r#"
---@param value string
---@return string
---@overload fun(value: integer): integer
local function accept(value) return value end
---@cast accept fun(value: string): string
accept(1)
"#;
    let diagnostics = check_source(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
}

#[test]
fn composed_overload_keeps_overloads_after_an_unrelated_cast() {
    let source = format!(
        "{DECLARATION}\nlocal other = 'text'\n---@cast other string\naccept({{count = 1}}, 1)\nreturn other"
    );
    let diagnostics = check_source(&source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn composed_overload_respects_inline_primary_signature_assertion() {
    let source = r#"
---@param value string
---@return string
---@overload fun(value: integer): integer
local function accept(value) return value end
accept --[[@as fun(value: string): string]] (1)
"#;
    let diagnostics = check_source(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
}

#[test]
fn composed_overload_dictionary_aliases_keep_complete_call_matching() {
    let declaration = format!(
        "---@alias StringIntegers table<string, integer>\n{}",
        DECLARATION.replace(
            "fun(values: table<string, integer>, mode: integer)",
            "fun(values: StringIntegers, mode: integer)",
        )
    );
    let valid = check_source(&format!("{declaration}accept({{count = 1}}, 1)"));
    assert!(valid.is_empty(), "{valid:?}");
    let invalid = check_source(&format!("{declaration}accept({{count = 1}}, 'a')"));
    assert_eq!(invalid.len(), 1, "{invalid:?}");
    assert_eq!(invalid[0].code, DiagnosticCode::ParamTypeMismatch);
    assert!(usize::from(invalid[0].range.start()) >= declaration.len());
}

#[test]
fn composed_overload_bad_alias_keeps_both_diagnostic_contracts() {
    let source = "---@alias StringIntegers table<string, integer>\n---@param values table<integer, string>\n---@param mode string\n---@overload fun(values: StringIntegers, mode: integer)\nlocal function accept(values, mode) return values, mode end\naccept({count = 'bad'}, 1)";
    let diagnostics = check_source(source);
    assert_eq!(
        diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
        vec![
            DiagnosticCode::ParamTypeMismatch,
            DiagnosticCode::AssignTypeMismatch
        ],
        "{diagnostics:?}"
    );
    assert_eq!(&source[diagnostics[0].range], "{count = 'bad'}");
    assert_eq!(&source[diagnostics[1].range], "'bad'");
}

#[test]
fn composed_overload_user_table_class_keeps_the_known_field_warning() {
    let source = "---@class table<K, V>\n---@field marker string\nlocal _Table = {}\n---@param values table<string, integer>\nlocal function accept(values) return values end\naccept({marker = 'ok'})";
    let diagnostics = check_source(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::AssignTypeMismatch);
    assert_eq!(&source[diagnostics[0].range], "'ok'");
}
