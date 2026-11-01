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
