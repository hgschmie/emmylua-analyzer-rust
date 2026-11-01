use crate::{DiagnosticCode, VirtualWorkspace};

fn check(body: &str, safe: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let src = format!(
        r#"
---@param rows integer[][]
---@param key integer
---@param other integer
---@param predicate fun():boolean
local function f(rows, key, other, predicate)
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
fn computed_facts_survive_opaque_calls_and_child_writes() {
    for body in [
        "if rows[key] then predicate(); print(rows[key][1]) end",
        "rows[key] = rows[key] or {}; local v = rows[key][other] or predicate() or 0; rows[key][other] = v",
        "if rows[key] then rows[key][other] = nil; print(rows[key][1]) end",
        "if rows[key] then local out = {}; out[other] = 1; print(rows[key][1]) end",
        "if rows[key] then local deferred = function() rows[key] = nil end; print(rows[key][1]) end",
    ] {
        check(body, true);
    }
}

#[test]
fn global_dictionary_initialization_survives_callback() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::NeedCheckNil,
        r#"
---@class Storage
---@field capacity table<string, table<string, number>?>
---@type Storage
storage = {}
---@param name string
---@param quality string
---@param callback fun():number?
local function capacity(name, quality, callback)
    storage.capacity[name] = storage.capacity[name] or {}
    local value = storage.capacity[name][quality] or callback() or 0
    storage.capacity[name][quality] = value
    return math.floor(value)
end
"#
    ));
}

#[test]
fn computed_guards_reestablished_inside_loops_survive_later_writes() {
    for body in [
        "for i=1,10 do if rows[key] then print(rows[key][1]); rows[key]=nil end end",
        "while predicate() do if rows[key] then print(rows[key][1]); rows[key]=nil end end",
        "repeat if rows[key] then print(rows[key][1]); rows[key]=nil end until predicate()",
    ] {
        check(body, true);
    }
}

#[test]
fn computed_facts_cross_nonconflicting_nested_loop() {
    check(
        r#"
local result = {}
for i = 1, 32 do
    if rows[i] and predicate() then
        for id in pairs(rows[i]) do
            if predicate() then result[id] = 1
            else rows[i][id] = nil end
        end
    end
end
return result
"#,
        true,
    );
    check(
        "rows[key] = {}; for id = 1, 10 do rows[key][id] = id end",
        true,
    );
}

#[test]
fn computed_facts_stop_at_visible_conflicts() {
    for body in [
        "if rows[key] then rawset(rows, key, nil); print(rows[key][1]) end",
        "if rows[key] then rows[key][other] = rawset(rows, key, nil); print(rows[key][1]) end",
        "rows[key] = {}; rawset(rows, key, nil); print(rows[key][1])",
        "rows[key] = {}; for i = 1, 10 do print(rows[key][1]); rows[key] = nil end",
        "local function clear() rows[key] = nil end; rows[key] = {}; clear(); print(rows[key][1])",
        "if rows[key] then setmetatable(rows, {}); print(rows[key][1]) end",
        "if rows[key] then local other = {}; other = rows; other[key] = nil; print(rows[key][1]) end",
        "if rows[key] then table.remove(rows, key); print(rows[key][1]) end",
        "local function clear() rows[key] = nil end; if rows[key] then clear(); print(rows[key][1]) end",
        "local clear = function() rows[key] = nil end; if rows[key] then clear(); print(rows[key][1]) end",
        "if rows[key] then (function() rows[key] = nil end)(); print(rows[key][1]) end",
        "if rows[key] then for i = 1, 10 do print(rows[key][1]); rows[key] = nil end end",
        "if rows[key] then for i = 1, 10 do print(rows[key][1]); key = other end end",
        "local alias = rows; if rows[key] then for i = 1, 10 do print(rows[key][1]); alias[key] = nil end end",
        "local out = {}; escape(out); if rows[key] then out[other] = nil; print(rows[key][1]) end",
        "if rows[key] then for i = 1, 10 do print(rows[key][1]); rawset(rows, key, nil) end end",
    ] {
        check(body, false);
    }
}
