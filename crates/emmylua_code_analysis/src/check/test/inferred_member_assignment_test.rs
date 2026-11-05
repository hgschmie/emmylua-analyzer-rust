use crate::{DiagnosticCode, VirtualWorkspace};

#[test]
fn inferred_member_first_assignment_does_not_check_against_its_own_initializer() {
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

#[test]
fn inferred_member_assignment_preserves_union_and_index_contracts() {
    for (prefix, declaration, accepted) in [
        ("Entity|Other", "---@field offsets number[]", true),
        ("Entity|Other", "---@field offsets string[]", false),
        ("Entity", "---@field [string] number[]", true),
        ("Entity", "---@field [string] string[]", false),
        ("table<string, string[]>", "", false),
    ] {
        let source = format!(
            r#"
            ---@class Constants
            ---@field offsets number[]
            local constants = {{offsets = {{0, 0}}}}
            ---@class Entity
            {declaration}
            local Entity = {{}}
            ---@class Other
            {declaration}
            local Other = {{}}
            ---@param p {prefix}
            local function assign(p)
                p.offsets = constants.offsets
            end
            return assign, Entity, Other
        "#
        );
        let mut ws = VirtualWorkspace::new();
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
            accepted,
            "prefix={prefix}, {declaration}, expected accepted={accepted}\n{source}"
        );
    }
}

#[test]
fn inferred_member_assignment_preserves_partial_union_and_disjoint_index_contracts() {
    for (prefix, declaration, accepted) in [
        ("Entity|Other", "---@field offsets number[]", true),
        ("Other|Entity", "---@field offsets string[]", false),
        ("Entity|Other", "---@field offsets string[]", false),
        (
            "Entity",
            "---@field [integer] number[]\n---@field [string] string[]",
            false,
        ),
        (
            "Entity",
            "---@field [string] string[]\n---@field [integer] number[]",
            false,
        ),
        (
            "Entity",
            "---@field [integer] string[]\n---@field [string] number[]",
            true,
        ),
    ] {
        let source = format!(
            r#"
            ---@class Entity
            {declaration}
            local Entity = {{}}
            ---@class Other
            ---@field name string
            local Other = {{}}
            ---@param p {prefix}
            ---@param replacement number[]
            local function assign(p, replacement)
                p.offsets = replacement
            end
            return assign, Entity, Other
        "#
        );
        let mut ws = VirtualWorkspace::new();
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
            accepted,
            "prefix={prefix}, {declaration}, expected accepted={accepted}\n{source}"
        );
    }
}
