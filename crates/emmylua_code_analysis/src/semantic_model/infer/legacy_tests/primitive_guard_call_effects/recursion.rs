use super::*;

#[test]
fn visible_writes_in_large_callees_invalidate_member_guards() {
    for (write, nullable) in [
        ("if flag then elem[key] = nil end", true),
        ("if flag then other[key] = nil end", false),
    ] {
        let filler = "local ignored = 1\n".repeat(700);
        let body = format!(
            "local function large()\n{write}\n{filler}end\nlarge()\nlocal value = elem.handler.name -- observe"
        );
        assert_primitive_call_boundary(&body, nullable);
    }
}

#[test]
fn visible_writes_in_deep_callees_invalidate_member_guards() {
    for (write, nullable) in [
        ("if flag then elem[key] = nil end", true),
        ("if flag then other[key] = nil end", false),
    ] {
        let mut body = format!("local function step0() {write} end\n");
        for index in 1..12 {
            body.push_str(&format!(
                "local function step{index}() step{}() end\n",
                index - 1
            ));
        }
        body.push_str("step11()\nlocal value = elem.handler.name -- observe");
        assert_primitive_call_boundary(&body, nullable);
    }
}

#[test]
fn recursive_visible_callees_keep_receiver_bindings_distinct() {
    for (body, nullable) in [
        (
            "local function recurse(receiver) if flag then recurse(receiver) end end\nrecurse(elem)\nlocal value = elem.handler.name -- observe",
            false,
        ),
        (
            "local function recurse(receiver) if flag then recurse(elem) end; receiver[key] = nil end\nrecurse(other)\nlocal value = elem.handler.name -- observe",
            true,
        ),
        (
            "local function recurse(receiver) if flag then recurse(other) end; receiver[key] = nil end\nrecurse(other)\nlocal value = elem.handler.name -- observe",
            false,
        ),
    ] {
        assert_primitive_call_boundary(body, nullable);
    }
}

#[test]
fn recursive_descendant_receivers_terminate_without_losing_captures() {
    for (body, nullable) in [
        (
            "local function descend(receiver) descend(receiver.child) end\ndescend(elem)\nlocal value = elem.handler.name -- observe",
            false,
        ),
        (
            "local function descend(receiver) if flag then descend(receiver.child) end; elem[key] = nil end\ndescend(other)\nlocal value = elem.handler.name -- observe",
            true,
        ),
        (
            "local function write(receiver) receiver = elem; receiver[key] = nil end\nlocal function descend(receiver) if flag then descend(receiver.child) else write(receiver) end end\ndescend(other)\nlocal value = elem.handler.name -- observe",
            true,
        ),
        (
            "local function descend(receiver) if flag then descend() end; other[key] = nil end\ndescend(elem)\nlocal value = elem.handler.name -- observe",
            false,
        ),
    ] {
        assert_primitive_call_boundary(body, nullable);
    }
}

#[test]
fn cyclic_receiver_aliases_terminate_and_keep_reachable_writes() {
    for (receiver, nullable) in [("elem", true), ("other", false)] {
        let body = format!(
            "local function redirect(target)\nlocal saved = target\nwhile flag do saved = saved.child end\nif flag then saved[key] = nil end\nend\nredirect({receiver})\nlocal value = elem.handler.name -- observe"
        );
        assert_primitive_call_boundary(&body, nullable);
    }
}

#[test]
fn recursive_member_callees_retain_external_entry_context() {
    for (last, nullable) in [("elem", true), ("other", false)] {
        assert_member_call_boundary(
            "type(elem.handler) == 'table'",
            "local methods = {}; function methods.clear(target) target[key] = nil end; local function run(m, target, next) if next then run(m, next, nil) else m.clear(target) end end",
            &format!("run(methods, other, {last})\nlocal value = elem.handler.name -- observe"),
            nullable,
        );
    }
}

#[test]
fn recursive_member_callees_do_not_revive_replaced_methods() {
    assert_member_call_boundary(
        "type(elem.handler) == 'table'",
        "local methods = {}; function methods.clear(target) target[key] = nil end; local function run(m, target, next) if next then m.clear = function() end; run(m, next, nil) else m.clear(target) end end",
        "run(methods, other, elem)\nlocal value = elem.handler.name -- observe",
        false,
    );
}
