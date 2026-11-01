use crate::{DiagnosticCode, VirtualWorkspace};
fn check(body: &str, safe: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let src = format!(
        r#"
---@class Row
---@field value number
---@class Owner
---@field rows Row[]
---@param owner Owner
---@param rows Row[]
---@param replacement Row[]
---@param key integer
---@param other integer
---@param flag boolean
local function f(owner, rows, replacement, key, other, flag)
{body}
end
"#
    );
    assert_eq!(
        ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &src),
        safe,
        "{body}"
    );
}
#[test]
fn computed_alias_guards_reach_original_lookup() {
    for body in [
        "local delivery = rows[key]; if delivery then print(rows[key].value) end",
        "local delivery = rows[key]; if not delivery then return end; print(rows[key].value)",
        "local delivery = rows[key]; if delivery ~= nil then print(rows[key].value) end",
        "local delivery = owner.rows[key]; if delivery then if flag then print('yes') end; print(owner.rows[key].value) end",
        "local delivery = rows[key]; if flag then print('yes') end; if delivery then print(rows[key].value) end",
        "for _, key in pairs({1, 2}) do local delivery = rows[key]; if delivery then if flag then print('yes') end; print(rows[key].value) end end",
    ] {
        check(body, true);
    }
}
#[test]
fn computed_guards_survive_all_merge_paths() {
    for body in [
        "if rows[key] then if flag then print('yes') end; print(rows[key].value) end",
        "if rows[key] then if flag then print('yes') else print('no') end; print(rows[key].value) end",
        "if flag then if not rows[key] then return end else if not rows[key] then return end end; print(rows[key].value)",
    ] {
        check(body, true);
    }
}
#[test]
fn computed_alias_and_merge_guards_reject_stale_facts() {
    for body in [
        "local delivery = rows[key]; rows[key] = nil; if delivery then print(rows[key].value) end",
        "local delivery = rows[key]; key = other; if delivery then print(rows[key].value) end",
        "local delivery = rows[key]; rows = replacement; if delivery then print(rows[key].value) end",
        "local delivery = owner.rows[key]; owner.rows = replacement; if delivery then print(owner.rows[key].value) end",
        "local delivery = rows[key]; delivery = rows[other]; if delivery then print(rows[key].value) end",
        "local delivery = rows[key]; if delivery then if flag then rows[key] = nil end; print(rows[key].value) end",
        "if rows[key] then if flag then key = other end; print(rows[key].value) end",
        "if flag then if not rows[key] then return end end; print(rows[key].value)",
        "local delivery = rows[key]; if delivery then rows[key] = nil; print(rows[key].value) end",
        "local delivery = rows[key]; local function clear() rows[key] = nil end; clear(); if delivery then print(rows[key].value) end",
        "local delivery = rows[key]; rawset(rows, key, nil); if delivery then print(rows[key].value) end",
        "local delivery = rows[key]; if delivery then local later = function() print(rows[key].value) end end",
        "local delivery, key = rows[key], other; if delivery then print(rows[key].value) end",
    ] {
        check(body, false);
    }
}
#[test]
fn computed_merge_search_reuses_shared_predecessors() {
    let branches = "if flag then print('yes') else print('no') end;".repeat(32);
    check(
        &format!("if rows[key] then {branches} print(rows[key].value) end"),
        true,
    );
}

#[test]
fn computed_alias_guard_does_not_hide_later_deletion_of_entry() {
    check(
        "local delivery = rows[key]; if delivery then rows[key] = nil; print(delivery.value) end",
        true,
    );
}

#[test]
fn computed_alias_guards_keep_initializer_and_guard_order() {
    for body in [
        "if rows[key] then local delivery=rows[key]; if delivery then print(rows[key].value) end end",
        "local delivery=rows[key]; if rows[key] then print(rows[key].value) end",
        "local delivery=rows[key]; if delivery then print(rows[key].value) end",
    ] {
        check(body, true);
    }
    for body in [
        "if rows[key] then local delivery=rows[key]; rows[key]=nil; if delivery then print(rows[key].value) end end",
        "local delivery=rows[key]; rows[key]=nil; if delivery then print(rows[key].value) end",
    ] {
        check(body, false);
    }
}

#[test]
fn computed_guards_follow_table_move_destination() {
    for (call, safe) in [
        ("table.move(rows, 1, 2, 3)", false),
        ("table.move(rows, 1, 2, 3, rows)", false),
        ("table.move(replacement, 1, 2, 3, rows)", false),
        ("table.move(rows, 1, 2, 3, replacement)", true),
    ] {
        check(
            &format!("if rows[key] then {call}; print(rows[key].value) end"),
            safe,
        );
    }
}
