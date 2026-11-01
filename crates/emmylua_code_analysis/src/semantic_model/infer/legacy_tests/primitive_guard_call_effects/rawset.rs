use super::*;

#[test]
fn primitive_guard_call_boundary_rawset_same_field() {
    assert_primitive_call_boundary(
        r#"
rawset(elem, "handler", nil)
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_rawset_other_field() {
    assert_primitive_call_boundary(
        r#"
rawset(elem, "other", nil)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_rawset_other_receiver() {
    assert_primitive_call_boundary(
        r#"
rawset(other, "handler", nil)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_shadowed_rawset() {
    assert_primitive_call_boundary(
        r#"
local function rawset(target, field, value) return target end
rawset(elem, 'handler', nil)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn rawset_dynamic_and_constant_keys_invalidate_the_same_receiver() {
    for key in ["key", "'handler'"] {
        assert_primitive_call_boundary(
            &format!("rawset(elem, {key}, nil)\nlocal value = elem.handler.name -- observe"),
            true,
        );
    }
}

#[test]
fn global_rawset_replacements_do_not_invent_native_writes() {
    for setup in ["rawset = function(...) end", "function rawset(...) end"] {
        assert_member_call_boundary(
            "type(elem.handler) == 'table'",
            setup,
            "rawset(elem, 'handler', nil)\nlocal value = elem.handler.name -- observe",
            false,
        );
    }
}

#[test]
fn named_rawset_replacements_keep_possible_native_and_visible_custom_writes() {
    assert_member_call_boundary(
        "type(elem.handler) == 'table'",
        "if flag then function rawset(...) end end",
        "rawset(elem, 'handler', nil)\nlocal value = elem.handler.name -- observe",
        true,
    );
    assert_member_call_boundary(
        "type(elem.handler) == 'table'",
        "function rawset(target, key, value) target.handler = nil end",
        "rawset(elem, 'unrelated', nil)\nlocal value = elem.handler.name -- observe",
        true,
    );
}

#[test]
fn rawset_union_keys_can_target_named_fields() {
    for (key_type, nullable) in [("string|number", true), ("number", false)] {
        assert_member_call_boundary(
            "type(elem.handler) == 'table'",
            &format!("---@type {key_type}\nlocal mixed = opaque()"),
            "rawset(elem, mixed, nil)\nlocal value = elem.handler.name -- observe",
            nullable,
        );
    }
}
