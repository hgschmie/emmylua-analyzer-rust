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
    for (declaration, setup, accepted) in [
        ("---@field offsets number[]", "", true),
        ("---@field offsets string[]", "", false),
        ("", "---@type string[]\np.offsets = {}", false),
        ("", "p.offsets = {'original'}", false),
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
            "{source}"
        );
    }
}
