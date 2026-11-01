use super::*;

#[test]
fn fixed_member_callees_invalidate_the_written_receiver() {
    for body in [
        "local methods = {}\nfunction methods.clear(receiver) if flag then receiver[key] = nil end end\nmethods.clear(elem)",
        "function elem:clear() if flag then self[key] = nil end end\nelem:clear()",
        "local methods = {}\nfunction methods:clear() if flag then self[key] = nil end end\nmethods.clear(elem)",
        "local methods = {}\nfunction methods:clear(receiver) if flag then receiver[key] = nil end end\nmethods:clear(elem)",
    ] {
        assert_primitive_call_boundary(
            &format!("{body}\nlocal value = elem.handler.name -- observe"),
            true,
        );
    }
}

#[test]
fn fixed_member_callees_preserve_unrelated_receiver_guards() {
    for body in [
        "local methods = {}\nfunction methods:clear() if flag then self[key] = nil end end\nmethods:clear(elem)",
        "function other:clear() if flag then self[key] = nil end end\nother:clear()",
        "function elem:clear(receiver) if flag then receiver[key] = nil end end\nelem:clear(other)",
        "local methods = {}\nfunction methods.clear(receiver) if flag then receiver[key] = nil end end\nmethods:clear(elem)",
    ] {
        assert_primitive_call_boundary(
            &format!("{body}\nlocal value = elem.handler.name -- observe"),
            false,
        );
    }
}

#[test]
fn fixed_member_callees_follow_reaching_assignments_and_full_paths() {
    for (setup, body, nullable) in [
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "methods.clear(elem)",
            true,
        ),
        (
            "local methods = {}; methods.clear = function(receiver) receiver[key] = nil end",
            "methods.clear(elem)",
            true,
        ),
        (
            "local methods = {clear = function(receiver) receiver[key] = nil end}",
            "methods.clear(elem)",
            true,
        ),
        (
            "local methods = {nested = {clear = function(receiver) receiver[key] = nil end}}",
            "methods.nested.clear(elem)",
            true,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "methods.clear = function() end; methods.clear(elem)",
            false,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "methods = {}; methods.clear(elem)",
            false,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "methods[key] = opaque; methods.clear(elem)",
            false,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "if flag then methods.clear = function() end end; methods.clear(elem)",
            true,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "if flag then methods.clear = function() end else methods.clear = opaque end; methods.clear(elem)",
            false,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "methods.clear(other)",
            false,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "do local methods = {}; function methods.clear() end; methods.clear(elem) end",
            false,
        ),
        (
            "local methods = {left = {}, right = {}}; function methods.left.clear(receiver) receiver[key] = nil end; function methods.right.clear() end",
            "methods.right.clear(elem)",
            false,
        ),
        (
            "local methods = {left = {}, right = {}}; function methods.left.clear(receiver) receiver[key] = nil end; function methods.right.clear() end",
            "methods.left.clear(elem)",
            true,
        ),
        (
            "local methods = {child = {}}; function methods.child.clear(receiver) receiver[key] = nil end",
            "methods.child = {}; methods.child.clear(elem)",
            false,
        ),
        (
            "local methods = {}; function methods.clear() end",
            "methods.clear(elem); function methods.clear(receiver) receiver[key] = nil end",
            false,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
            "local saved = methods.clear; methods.clear = function() end; saved(elem)",
            true,
        ),
    ] {
        assert_member_call_boundary(
            "type(elem.handler) == 'table'",
            setup,
            &format!("{body}\nlocal value = elem.handler.name -- observe"),
            nullable,
        );
    }
}

#[test]
fn fixed_member_callees_forward_tables_at_the_invocation_state() {
    for (setup, body, nullable) in [
        (
            "local methods = {clear = function(receiver) receiver[key] = nil end}; local function helper(m) m.clear(elem) end",
            "helper(methods)",
            true,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end; local function helper(m) m.clear(elem) end",
            "helper(methods)",
            true,
        ),
        (
            "local methods = {}; local function helper(m) m.clear(elem) end; function methods.clear(receiver) receiver[key] = nil end",
            "helper(methods)",
            true,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end; local function helper(m) m.clear(elem) end",
            "methods.clear = function() end; helper(methods)",
            false,
        ),
        (
            "local methods = {}; function methods.clear() end; local function helper(m) m.clear(elem) end",
            "helper(methods); function methods.clear(receiver) receiver[key] = nil end; helper(methods)",
            true,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end; local function inner(m) m.clear(elem) end; local function helper(m) inner(m) end",
            "helper(methods)",
            true,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end; local function inner(m) m.clear(elem) end; local function helper(m) m.clear = function() end; inner(m) end",
            "helper(methods)",
            false,
        ),
        (
            "local methods = {}; function methods.clear() end; local function helper() methods.clear(elem) end",
            "function methods.clear(receiver) receiver[key] = nil end; helper()",
            true,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end; local function helper() methods.clear(elem) end",
            "methods.clear = function() end; helper()",
            false,
        ),
        (
            "local methods = {}; function methods.clear(receiver) receiver[key] = nil end; local function helper(m) if flag then helper(m.child) end; m.clear(elem) end",
            "helper(methods)",
            true,
        ),
        (
            "local methods = {}; function methods.clear() end; local function helper(m) if flag then helper(m.child) end; m.clear(elem) end",
            "helper(methods)",
            false,
        ),
    ] {
        assert_member_call_boundary(
            "type(elem.handler) == 'table'",
            setup,
            &format!("{body}\nlocal value = elem.handler.name -- observe"),
            nullable,
        );
    }
}

#[test]
fn descendant_writes_do_not_replace_the_ancestor_callee() {
    assert_member_call_boundary(
        "type(elem.handler) == 'table'",
        "local methods = {}; function methods.clear(receiver) receiver[key] = nil end",
        "methods.clear.child = opaque; methods.clear(elem)\nlocal value = elem.handler.name -- observe",
        true,
    );
}

#[test]
fn mixed_and_sequence_constructor_callees_work_through_local_aliases() {
    for installation in [
        "local methods = {1, clear = writer}; local action = methods.clear",
        "local methods = {writer}; local action = methods[1]",
        "local methods = {}; methods.clear = writer; local action = methods.clear",
    ] {
        for (receiver, nullable) in [("elem", true), ("other", false)] {
            assert_member_call_boundary(
                "type(elem.handler) == 'table'",
                &format!("local function writer(target) target.handler = nil end\n{installation}"),
                &format!("action({receiver})\nlocal value = elem.handler.name -- observe"),
                nullable,
            );
        }
    }
}
