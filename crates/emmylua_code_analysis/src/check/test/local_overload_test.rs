use super::check_source;
use crate::DiagnosticCode;

const DECLARATION: &str = r#"
---@param value string
---@return string
---@overload fun(value: integer): integer
local function accept(value) return value end
"#;

#[test]
fn local_overload_review_incomplete_dictionary_field_keeps_earlier_value() {
    let source = "---@param values table<string, integer>\nlocal function accept(values) return values end\naccept({count = 'bad', count = })";
    let diagnostics = check_source(source);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::ParamTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn local_overload_accepts_primary_and_documented_alternative() {
    for call in ["accept('text')", "accept(1)"] {
        let diagnostics = check_source(&format!("{DECLARATION}{call}"));
        assert!(diagnostics.is_empty(), "{call}: {diagnostics:?}");
    }
}

#[test]
fn local_overload_rejects_argument_outside_all_signatures() {
    let diagnostics = check_source(&format!("{DECLARATION}accept(true)"));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
}

#[test]
fn local_overload_respects_explicit_function_type() {
    let source = "---@type fun(value: string): string\n---@overload fun(value: integer): integer\nlocal accept = function(value) return value end\naccept(1)";
    let diagnostics = check_source(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
}

#[test]
fn local_overload_does_not_restore_reassigned_overloads() {
    let source = format!(
        "{DECLARATION}\n---@param value string\n---@return string\nlocal function replacement(value) return value end\naccept = replacement\naccept(1)"
    );
    let diagnostics = check_source(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
}

#[test]
fn local_overload_respects_cast_to_the_primary_signature() {
    let source = format!("{DECLARATION}\n---@cast accept fun(value: string): string\naccept(1)");
    let diagnostics = check_source(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
}

#[test]
fn local_overload_respects_inline_primary_signature_assertion() {
    let source = format!("{DECLARATION}\naccept --[[@as fun(value: string): string]] (1)");
    let diagnostics = check_source(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
}

#[test]
fn local_overload_keeps_overloads_after_unrelated_casts() {
    let source = format!(
        "{DECLARATION}\nlocal other = 'text'\n---@cast other string\naccept(1)\nreturn other"
    );
    let diagnostics = check_source(&source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn local_overload_distinguishes_shadowed_cast_declarations() {
    let source = format!(
        "{DECLARATION}\ndo\nlocal accept = 'text'\n---@cast accept string\nlocal _ = accept\nend\naccept(1)"
    );
    let diagnostics = check_source(&source);
    let mismatches = diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::ParamTypeMismatch)
        .collect::<Vec<_>>();
    assert!(mismatches.is_empty(), "{diagnostics:?}");
}

#[test]
fn local_overload_dictionary_calls_require_one_complete_signature() {
    let declaration = "---@param values table<integer, string>\n---@param mode string\n---@overload fun(values: table<string, integer>, mode: integer)\nlocal function accept(values, mode) return values, mode end\n";
    for call in ["accept({'a'}, 'a')", "accept({count = 1}, 1)"] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert!(diagnostics.is_empty(), "{call}: {diagnostics:?}");
    }
    for call in ["accept({count = 1}, 'a')", "accept({'a'}, 1)"] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert_eq!(diagnostics.len(), 1, "{call}: {diagnostics:?}");
        assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
        assert!(usize::from(diagnostics[0].range.start()) >= declaration.len());
    }
    let valid = "accept({'a'}, 'a')\naccept({count = 1}, 1)\n";
    let diagnostics = check_source(&format!("{declaration}{valid}accept({{count = 1}}, 'a')"));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
    assert!(usize::from(diagnostics[0].range.start()) >= declaration.len() + valid.len());
}

#[test]
fn local_overload_dictionary_checks_keep_nil_overwrites_and_return_slots() {
    let declaration = "---@param values table<string, integer>\nlocal function accept(values) return values end\n";
    for argument in [
        "{count = 1}",
        "{count = nil}",
        "{count = 'bad', count = 1}",
        "{count = 'bad', count = nil}",
    ] {
        let diagnostics = check_source(&format!("{declaration}accept({argument})"));
        assert!(
            diagnostics
                .iter()
                .all(|d| d.code != DiagnosticCode::ParamTypeMismatch),
            "{argument}: {diagnostics:?}"
        );
    }
    for argument in ["{count = 'bad'}", "{1}", "{count = 1, count = 'bad'}"] {
        let diagnostics = check_source(&format!("{declaration}accept({argument})"));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::ParamTypeMismatch),
            "{argument}: {diagnostics:?}"
        );
    }
    let source = "---@return integer, string\nlocal function values() return 1, 'bad' end\n---@param items table<integer, integer>\nlocal function accept(items) return items end\naccept({values()})\naccept({(values())})";
    let diagnostics = check_source(source);
    let mismatches = diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::ParamTypeMismatch)
        .collect::<Vec<_>>();
    assert_eq!(mismatches.len(), 1, "{diagnostics:?}");
    assert!(usize::from(mismatches[0].range.end()) <= source.find("accept({(values())})").unwrap());
    let overwritten_tail = "---@return integer, integer\nlocal function values() return 1, 2 end\n---@param items table<integer, integer>\nlocal function accept(items) return items end\naccept({[2] = 'bad', values()})";
    let diagnostics = check_source(overwritten_tail);
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ParamTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn local_overload_scalar_calls_require_one_complete_signature() {
    let declaration = "---@param value string\n---@param mode string\n---@overload fun(value: integer, mode: integer)\nlocal function accept(value, mode) return value, mode end\n";
    for call in ["accept('text', 'text')", "accept(1, 1)"] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert!(diagnostics.is_empty(), "{call}: {diagnostics:?}");
    }
    for call in ["accept(1, 'text')", "accept('text', 1)"] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert_eq!(diagnostics.len(), 1, "{call}: {diagnostics:?}");
        assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
    }
}

#[test]
fn local_overload_dictionary_aliases_preserve_key_and_value_checks() {
    let declaration = "---@alias StringIntegers table<string, integer>\n---@param values table<integer, string>\n---@param mode string\n---@overload fun(values: StringIntegers, mode: integer)\nlocal function accept(values, mode) return values, mode end\n";
    let diagnostics = check_source(&format!("{declaration}accept({{count = 1}}, 1)"));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for call in ["accept({count = 1}, 'a')", "accept({count = 'bad'}, 1)"] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        let mismatches = diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::ParamTypeMismatch)
            .collect::<Vec<_>>();
        assert_eq!(mismatches.len(), 1, "{call}: {diagnostics:?}");
        assert!(usize::from(mismatches[0].range.start()) >= declaration.len());
    }
}

#[test]
fn local_overload_dictionary_checks_do_not_replace_a_user_table_class() {
    let source = "---@class table<K, V>\n---@field marker string\nlocal _Table = {}\n---@param values table<string, integer>\nlocal function accept(values) return values end\naccept({marker = 'ok'})";
    let diagnostics = check_source(source);
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ParamTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn local_overload_later_cast_does_not_change_an_earlier_call() {
    let earlier = format!("{DECLARATION}accept(1)\n");
    let source = format!("{earlier}---@cast accept fun(value: string): string\naccept(1)");
    let diagnostics = check_source(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
    assert!(usize::from(diagnostics[0].range.start()) >= earlier.len());
}

#[test]
fn local_overload_parenthesized_dictionaries_require_a_complete_signature() {
    let declaration = "---@param values table<integer, string>\n---@param mode string\n---@overload fun(values: table<string, integer>, mode: integer)\nlocal function accept(values, mode) return values, mode end\n";
    for call in ["accept(({'a'}), 'a')", "accept((({count = 1})), 1)"] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert!(diagnostics.is_empty(), "{call}: {diagnostics:?}");
    }
    for call in ["accept(({count = 1}), 'a')", "accept((({'a'})), 1)"] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert_eq!(diagnostics.len(), 1, "{call}: {diagnostics:?}");
        assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
        assert!(usize::from(diagnostics[0].range.start()) >= declaration.len());
    }
}

#[test]
fn local_overload_forwarded_dictionary_calls_require_a_complete_signature() {
    let declaration = "---@param values table<integer, string>\n---@param mode string\n---@overload fun(values: table<string, integer>, mode: integer)\nlocal function accept(values, mode) return values, mode end\n";
    for call in [
        "pcall(accept, ({'a'}), 'a')",
        "pcall(accept, ({count = 1}), 1)",
    ] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert!(diagnostics.is_empty(), "{call}: {diagnostics:?}");
    }
    for call in [
        "pcall(accept, ({count = 1}), 'a')",
        "pcall(accept, ({'a'}), 1)",
    ] {
        let diagnostics = check_source(&format!("{declaration}{call}"));
        assert_eq!(diagnostics.len(), 1, "{call}: {diagnostics:?}");
        assert_eq!(diagnostics[0].code, DiagnosticCode::ParamTypeMismatch);
        assert!(usize::from(diagnostics[0].range.start()) >= declaration.len());
    }
}

#[test]
fn local_overload_dictionary_assertions_remain_authoritative() {
    let declaration = "---@param values table<string, integer>\nlocal function accept(values) return values end\n";
    for argument in [
        "{count = 'bad'} --[[@as table<string, integer>]]",
        "({count = 'bad'} --[[@as table<string, integer>]])",
        "(({count = 'bad'}) --[[@as table<string, integer>]])",
    ] {
        let diagnostics = check_source(&format!("{declaration}accept({argument})"));
        assert!(
            diagnostics
                .iter()
                .all(|d| d.code != DiagnosticCode::ParamTypeMismatch),
            "{argument}: {diagnostics:?}"
        );
    }
}
