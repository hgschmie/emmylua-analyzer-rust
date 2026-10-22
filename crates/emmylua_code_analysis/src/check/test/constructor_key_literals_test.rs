use crate::{DiagnosticCode, VirtualWorkspace};

#[test]
fn constructor_keys_preserve_string_literals() {
    for strict_index in [false, true] {
        for entry in [
            "startup = VALUE",
            "['startup'] = VALUE",
            r#"["startup"] = VALUE"#,
            r"['\x73tartup'] = VALUE",
            "[('startup')] = VALUE",
        ] {
            for (value, accepted) in [("'ok'", true), ("false", false)] {
                let mut ws = VirtualWorkspace::new();
                let mut config = ws.get_emmyrc();
                config.strict.table_index = strict_index;
                config.strict.doc_base_const_match_base_type = false;
                ws.update_emmyrc(config);
                let entry = entry.replace("VALUE", value);
                let source = format!(
                    "---@type {{ ['startup'|'runtime']: string }}\nlocal settings = {{ {entry} }}"
                );
                assert_eq!(
                    ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
                    accepted,
                    "strict_index={strict_index}: {source}"
                );
            }
        }
    }
}

#[test]
fn constructor_keys_preserve_integer_literals() {
    for strict_index in [false, true] {
        for entry in [
            "[1] = VALUE",
            "[0x1] = VALUE",
            "[1.0] = VALUE",
            "VALUE",
            "[(1)] = VALUE",
        ] {
            for (value, accepted) in [("'ok'", true), ("false", false)] {
                let mut ws = VirtualWorkspace::new();
                let mut config = ws.get_emmyrc();
                config.strict.table_index = strict_index;
                config.strict.doc_base_const_match_base_type = false;
                ws.update_emmyrc(config);
                let entry = entry.replace("VALUE", value);
                let source =
                    format!("---@type {{ [1|2]: string }}\nlocal settings = {{ {entry} }}");
                assert_eq!(
                    ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
                    accepted,
                    "strict_index={strict_index}: {source}"
                );
            }
        }
    }
}
