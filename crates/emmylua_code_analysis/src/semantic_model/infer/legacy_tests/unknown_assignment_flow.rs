use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaNameExpr};
use tokio_util::sync::CancellationToken;

use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

fn assert_flow(source: &str, expected_warnings: usize) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::UnnecessaryIf);
    let mut offset = 0;
    let expectations = source
        .split_inclusive('\n')
        .filter_map(|line| {
            let start = offset;
            offset += line.len();
            line.split_once("-- expect: ")
                .map(|(_, ty)| (start, offset, ws.ty(ty.trim())))
        })
        .collect::<Vec<_>>();
    let file = ws.def(source);
    let diagnostics = ws
        .analysis
        .diagnose_file(file, CancellationToken::new())
        .unwrap();
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    for (marker, _) in source.match_indices("-- rhs: unknown") {
        let line_start = source[..marker].rfind('\n').map_or(0, |start| start + 1);
        let assignment = chunk
            .descendants::<LuaAssignStat>()
            .find(|assignment| {
                let pos = u32::from(assignment.get_position()) as usize;
                pos >= line_start && pos < marker
            })
            .unwrap();
        let (_, values) = assignment.get_var_and_expr_list();
        assert_eq!(values.len(), 1);
        let value = &values[0];
        assert_eq!(
            model.type_of_expr_at(value.get_syntax_id(), value.get_position()),
            LuaType::Unknown,
            "unknown RHS precondition: {}",
            value.get_text()
        );
    }
    assert_eq!(diagnostics.len(), expected_warnings, "{diagnostics:#?}");
    for (start, end, expected) in expectations {
        let names = chunk
            .descendants::<LuaNameExpr>()
            .filter(|name| {
                let pos = u32::from(name.get_position()) as usize;
                name.get_name_text().as_deref() == Some("id") && pos >= start && pos < end
            })
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 1, "one id expression per expectation");
        let name = &names[0];
        assert_eq!(
            model.type_of_expr_at(name.get_syntax_id(), name.get_position()),
            expected,
            "{}",
            &source[start..end]
        );
    }
}

fn reassignment(rhs: &str) -> String {
    format!(
        r#"
---@param id string?
---@param values table<string,unknown>
---@param key string
local function dispatch(id, values, key)
    if id then return id end
    local before = id -- expect: nil
    id = {rhs} -- rhs: unknown
    local after = id -- expect: string?
    if id then
        local guarded = id -- expect: string
        return guarded
    end
end
"#
    )
}

#[test]
fn unknown_assignment_invalidates_nil_guard_after_call() {
    assert_flow(&reassignment("unavailable()"), 0);
}

#[test]
fn unknown_assignment_invalidates_nil_guard_after_index() {
    assert_flow(&reassignment("values[key]"), 0);
}

#[test]
fn unknown_assignment_invalidates_nil_guard_after_index_alternatives() {
    assert_flow(&reassignment("values[key] or values[tostring(key)]"), 0);
}

#[test]
fn unknown_assignment_invalidates_nil_guard_in_recursive_tag_map() {
    assert_flow(
        r#"
---@alias Basic boolean|number|string|table<string,Basic>
---@alias Tags table<string,Basic>

---@param id string?
---@param tags Tags
---@param key string
local function dispatch(id, tags, key)
    if id then return id
    elseif type(tags.handler) == 'table' then
        id = tags.handler[key] or tags.handler[tostring(key)]
        local after = id
        if id then
            local guarded = id
            return guarded
        end
    end
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_does_not_hide_known_nil_assignment() {
    assert_flow(
        r#"
---@param id string?
local function dispatch(id)
    if id then return id end
    id = nil
    local after = id -- expect: nil
    if id then return id end
end
"#,
        1,
    );
}

#[test]
fn unknown_assignment_preserves_local_annotation() {
    assert_flow(
        r#"
---@type string?
local id = nil
if id then return id end
id = unavailable() -- rhs: unknown
local after = id -- expect: string?
if id then
    local guarded = id -- expect: string
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_preserves_later_cast() {
    assert_flow(
        r#"
---@param id string?
local function dispatch(id)
    ---@cast id nil
    local before = id -- expect: nil
    id = unavailable() -- rhs: unknown
    local after = id -- expect: string?
    ---@cast id integer
    local casted = id -- expect: integer
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_invalidates_earlier_cast() {
    assert_flow(
        r#"
local id = nil
---@cast id string
local before = id -- expect: string
id = unavailable() -- rhs: unknown
local after = id -- expect: unknown
"#,
        0,
    );
}

#[test]
fn unknown_assignment_invalidates_type_guard() {
    assert_flow(
        r#"
---@param id string|integer
local function dispatch(id)
    if type(id) == 'string' then
        local before = id -- expect: string
        id = unavailable() -- rhs: unknown
        local after = id -- expect: string|integer
    end
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_invalidates_loop_guard() {
    assert_flow(
        r#"
---@param id string?
local function dispatch(id)
    while not id do
        local before = id -- expect: nil
        id = unavailable() -- rhs: unknown
        local after = id -- expect: string?
        if id then return id end
    end
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_merges_with_known_nil_branch() {
    assert_flow(
        r#"
---@param id string?
---@param choose boolean
local function dispatch(id, choose)
    if id then return id end
    if choose then id = unavailable() else id = nil end
    local after = id -- expect: string?
    if id then return id end
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_retains_unknown_declaration() {
    assert_flow(
        r#"
local id = unavailable()
id = unavailable_again() -- rhs: unknown
local after = id -- expect: unknown
"#,
        0,
    );
}

#[test]
fn unknown_assignment_preserves_callable_contract_and_later_guard() {
    assert_flow(
        r#"
---@param id (fun():integer)?
local function dispatch(id)
    if id then return id() end
    id = unavailable() -- rhs: unknown
    local after = id -- expect: (fun():integer)?
    if id then
        local guarded = id -- expect: fun():integer
        return id()
    end
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_invalidates_unannotated_nil_initializer() {
    assert_flow(
        r#"
local id = nil
id = unavailable() -- rhs: unknown
local after = id -- expect: unknown
if id then return id end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_invalidates_unannotated_global_nil_initializer() {
    assert_flow(
        r#"
id = nil
id = unavailable() -- rhs: unknown
local after = id -- expect: unknown
if id then return id end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_invalidates_captured_nil_initializer() {
    assert_flow(
        r#"
local id = nil
local function update()
    id = unavailable() -- rhs: unknown
    local after = id -- expect: unknown
    if id then return id end
end
"#,
        0,
    );
}

#[test]
fn unknown_assignment_preserves_global_explicit_nil_contract() {
    assert_flow(
        r#"
---@type nil
id = nil
id = unavailable() -- rhs: unknown
local after = id -- expect: nil
if id then return id end
"#,
        1,
    );
}

#[test]
fn unknown_assignment_preserves_explicit_nil_contract() {
    assert_flow(
        r#"
---@type nil
local id = nil
id = unavailable() -- rhs: unknown
local after = id -- expect: nil
if id then return id end
"#,
        1,
    );
}

#[test]
fn unknown_assignment_preserves_known_self_assignment() {
    assert_flow(
        r#"
---@param id string?
local function dispatch(id)
    if id then return id end
    id = id
    local after = id -- expect: nil
    if id then return id end
end
"#,
        1,
    );
}

#[test]
fn unknown_assignment_does_not_invalidate_guard_without_write() {
    assert_flow(
        r#"
---@param id string?
local function dispatch(id)
    if id then return id end
    unavailable()
    local after = id -- expect: nil
    if id then return id end
end
"#,
        1,
    );
}

#[test]
fn unknown_assignment_broad_indexes_do_not_keep_nil_guard() {
    for rhs in ["values[key]", "values[key] or values[tostring(key)]"] {
        // Broad table indexing need not infer Unknown, so check stale-guard
        // diagnostics without imposing the typed-index RHS precondition.
        let source = format!(
            r#"
---@param id string?
---@param values table
---@param key string
local function dispatch(id, values, key)
    if id then return id end
    local before = id -- expect: nil
    id = {rhs}
    local after = id
    if id then
        local guarded = id
        return guarded
    end
end
"#
        );
        assert_flow(&source, 0);
    }
}
