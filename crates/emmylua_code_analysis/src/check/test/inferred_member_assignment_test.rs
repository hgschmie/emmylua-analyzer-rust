use crate::{DiagnosticCode, VirtualWorkspace};

#[test]
fn inferred_member_first_assignment_uses_the_declared_source_contract() {
    for guarded in [false, true] {
        for index in ["offsets", "['offsets']"] {
            let target = if index.starts_with('[') {
                format!("p{index}")
            } else {
                format!("p.{index}")
            };
            let assignment = format!("{target} = constants.offsets");
            let body = if guarded {
                format!("if {target} then {assignment} end")
            } else {
                assignment
            };
            let source = format!(
                r#"
                ---@class Constants
                ---@field offsets number[]
                local constants = {{ offsets = {{ 0, 0, 0, 0 }} }}
                ---@class Entity
                ---@field name string
                local Entity = {{}}
                ---@param p Entity
                local function assign(p)
                    {body}
                end
                return assign, Entity
                "#
            );
            let mut ws = VirtualWorkspace::new();
            assert!(
                ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
                "{source}"
            );
        }
    }
}

#[test]
fn inferred_member_assignments_preserve_independent_destination_contracts() {
    for (case, declaration, setup, accepted) in [
        (
            "matching declared field",
            "---@field offsets number[]",
            "",
            true,
        ),
        (
            "incompatible declared field",
            "---@field offsets string[]",
            "",
            false,
        ),
        (
            "explicit assignment annotation",
            "",
            "---@type string[]\np.offsets = {}",
            false,
        ),
        (
            "previous inferred assignment",
            "",
            "p.offsets = {'original'}",
            false,
        ),
    ] {
        let source = format!(
            r#"
            ---@class Constants
            ---@field offsets number[]
            local constants = {{ offsets = {{ 0, 0, 0, 0 }} }}
            ---@class Entity
            ---@field name string
            {declaration}
            local Entity = {{}}
            ---@param p Entity
            local function assign(p)
                {setup}
                p.offsets = constants.offsets
            end
            return assign, Entity
            "#
        );
        let mut ws = VirtualWorkspace::new();
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
            accepted,
            "{case}: expected accepted={accepted}\n{source}"
        );
    }
}

#[test]
fn inferred_member_reassignment_after_typed_use_still_checks_the_original_contract() {
    let source = r#"
        ---@class Entity
        ---@field name string
        local Entity = {}
        ---@param p Entity
        ---@param replacement number[]
        local function assign(p, replacement)
            p.offsets = {'original'}
            ---@type string[]
            local used = p.offsets
            p.offsets = replacement
            return used
        end
        return assign, Entity
    "#;
    let diagnostics = super::check_source(source);
    let mismatches = diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::AssignTypeMismatch)
        .collect::<Vec<_>>();
    assert_eq!(mismatches.len(), 1, "{diagnostics:?}");
    let range = mismatches[0].range;
    assert_eq!(
        &source[usize::from(range.start())..usize::from(range.end())],
        "replacement"
    );
}
