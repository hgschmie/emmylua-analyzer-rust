use super::*;
use crate::{DiagnosticCode, VirtualWorkspace};

fn check(body: &str, safe: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let source = format!(
        r#"
            ---@class Group
            ---@field values integer[]
            ---@param groups Group[]
            ---@param key integer
            ---@param other_key integer
            ---@param other_groups Group[]
            ---@param maybe Group?
            ---@param flag boolean
            local function add(groups, key, other_key, other_groups, maybe, flag)
                {body}
            end
        "#
    );
    assert_eq!(
        ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
        safe,
        "{body}"
    );
}

#[test]
fn computed_index_assignment_narrows_read() {
    for body in [
        "groups[key] = groups[key] or { values = {} }; local values = groups[key].values",
        "groups[key] = { values = {} }; local values = groups[key].values",
        "groups[key] = { values = {} }; local unrelated = 1; local values = groups[key].values",
        "if flag then groups[key] = groups[key] or { values = {} }; local values = groups[key].values end",
        "for i = 1, 10 do groups[key] = groups[key] or { values = {} }; local values = groups[key].values end",
        "groups[key] = { values = {} }; print(groups[key].values)",
        "groups[key] = { values = {} }; return groups[key].values",
    ] {
        check(body, true);
    }
}

#[test]
fn computed_index_assignment_preserves_element_type() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def(
        r#"
            ---@class Group
            ---@field values integer[]
            ---@param groups Group[]
            ---@param key integer
            local function add(groups, key)
                groups[key] = groups[key] or { values = {} }
                local result = groups[key]
            end
        "#,
    );
    let model = ws.analysis.semantic_model(file);
    let index = model
        .chunk()
        .unwrap()
        .descendants::<LuaIndexExpr>()
        .last()
        .unwrap();
    let actual = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
    assert_eq!(ws.humanize_type(actual), "Group");
}

#[test]
fn computed_index_assignment_requires_same_stable_declarations() {
    for body in [
        "local values = groups[key].values",
        "groups[key] = {}; key = other_key; local values = groups[key].values",
        "groups[key] = {}; groups = other_groups; local values = groups[key].values",
        "groups[other_key] = {}; local values = groups[key].values",
        "other_groups[key] = {}; local values = groups[key].values",
        "groups[key] = {}; do local key = other_key; local values = groups[key].values end",
        "groups[key] = {}; do local groups = other_groups; local values = groups[key].values end",
        "groups[key], key = {}, other_key; local values = groups[key].values",
    ] {
        check(body, false);
    }
}

#[test]
fn computed_index_assignment_does_not_cross_uncertain_writes_or_control_flow() {
    for body in [
        "groups[key] = {}; groups[key] = nil; local values = groups[key].values",
        "groups[key] = {}; groups[key] = maybe; local values = groups[key].values",
        "groups[key] = maybe; local values = groups[key].values",
        "if flag then groups[key] = {} end; local values = groups[key].values",
        "while flag do groups[key] = {} end; local values = groups[key].values",
        "local alias = groups; groups[key] = {}; alias[key] = nil; local values = groups[key].values",
        "groups[key] = {}; local function later() local values = groups[key].values end",
    ] {
        check(body, false);
    }
}

#[test]
fn computed_index_assignment_survives_opaque_calls() {
    for body in [
        "groups[key] = {}; mutate(); local values = groups[key].values",
        "groups[key] = {}; local unused = mutate(); local values = groups[key].values",
        "groups[key] = {}; if mutate() then local values = groups[key].values end",
        "groups[key] = {}; print(mutate(), groups[key].values)",
        "groups[key] = {}; local unused, values = mutate(), groups[key].values",
    ] {
        check(body, true);
    }
}
