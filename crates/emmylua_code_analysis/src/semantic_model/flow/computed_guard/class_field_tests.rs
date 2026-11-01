use crate::{DiagnosticCode, VirtualWorkspace};
fn check(body: &str, safe: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let src = format!(
        r#"
---@class Record
---@field value integer
---@class Registry
---@field first Record?
---@field second Record?
---@alias Key 'first'|'second'
---@param registry Registry
---@param key Key
---@param other Key
local function f(registry, key, other)
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
fn computed_class_fields_keep_matching_guards() {
    for body in [
        "if registry[key] then mutate(); print(registry[key].value) end",
        "if registry[key] then print(registry[key].value) end",
        "local value = registry[key] and registry[key].value",
        "if not (registry[key] and registry[key].value) then return end",
    ] {
        check(body, true);
    }
}
#[test]
fn computed_class_fields_reject_stale_or_different_guards() {
    for body in [
        "print(registry[key].value)",
        "if registry[key] then key = other; print(registry[key].value) end",
        "if registry[key] then print(registry[other].value) end",
        "if registry[key] then registry[key] = nil; print(registry[key].value) end",
    ] {
        check(body, false);
    }
}
