use crate::{DiagnosticCode, VirtualWorkspace};

#[test]
fn computed_guards_start_after_earlier_calls_in_the_same_expression() {
    for (body, safe) in [
        ("local value = clear() and rows[key] and rows[key][1]", true),
        (
            "if clear() and rows[key] then local value=rows[key][1] end",
            true,
        ),
        (
            "if rows[key] and clear() then local value=rows[key][1] end",
            false,
        ),
        (
            "local value = rows[key] and clear() and rows[key][1]",
            false,
        ),
        (
            "local value = clear() and rows[key] and clear() and rows[key][1]",
            false,
        ),
        ("local value = rows[key] and harmless(rows[key][1])", true),
        ("local value = clear(rows[key] and rows[key][1])", true),
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let source = [
                "---@param rows integer[][]\n---@param key integer\nlocal function run(rows, key)\nlocal function clear() rows[key] = nil; return true end\nlocal function harmless(value) return value end\n",
                body, "\nend",
            ].concat();
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            safe,
            "{body}"
        );
    }
}
