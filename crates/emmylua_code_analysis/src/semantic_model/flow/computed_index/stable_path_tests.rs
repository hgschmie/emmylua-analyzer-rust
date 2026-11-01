use crate::{DiagnosticCode, VirtualWorkspace};
fn check(body: &str, safe: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let src = format!(
        r#"
---@class Group
---@field values integer[]
---@alias Groups Group[]
---@class Storage
---@field groups Group[]
---@param storage Storage
---@param other_storage Storage
---@param key integer
---@param other integer
---@param value Group?
local function f(storage, other_storage, key, other, value)
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
fn stable_nested_assignment_paths() {
    for body in [
        "storage.groups[key] = {}; mutate(); print(storage.groups[key].values)",
        "storage.groups[key] = storage.groups[key] or {}; local result = storage.groups[key]; print(result.values)",
        "local groups = storage.groups; groups[key] = groups[key] or {}; print(groups[key].values)",
        "storage.groups[key] = {}; local count = #storage.groups[key].values",
    ] {
        check(body, true);
    }
}
#[test]
fn stable_nested_assignment_invalidations() {
    for body in [
        "storage.groups[key] = {}; key = other; print(storage.groups[key].values)",
        "storage.groups[key] = {}; storage.groups = {}; print(storage.groups[key].values)",
        "storage.groups[key] = {}; storage = other_storage; print(storage.groups[key].values)",
        "storage.groups[key] = value; print(storage.groups[key].values)",
        "if key > 0 then storage.groups[key] = {} end; print(storage.groups[key].values)",
    ] {
        check(body, false);
    }
}
