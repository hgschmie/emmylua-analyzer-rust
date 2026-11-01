use super::*;

#[test]
fn primitive_guard_call_boundary_captured_receiver_alias() {
    assert_primitive_call_boundary(
        r#"
local alias = elem
local function clear_alias() alias[key] = nil end
clear_alias()
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_callee_alias() {
    assert_primitive_call_boundary(
        r#"
local action = clear
action()
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_reassigned_callee_is_opaque() {
    assert_primitive_call_boundary(
        r#"
local action = clear
action = opaque
action()
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_conditional_parameter_rebind() {
    assert_primitive_call_boundary(
        r#"
local function redirect(receiver)
    if flag then receiver = other end
    receiver[key] = nil
end
redirect(elem)
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_definite_parameter_rebind() {
    assert_primitive_call_boundary(
        r#"
local function redirect(receiver)
    receiver = other
    receiver[key] = nil
end
redirect(elem)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_parameter_alias_before_rebind() {
    assert_primitive_call_boundary(
        r#"
local function redirect(receiver)
    local saved = receiver
    receiver = other
    saved[key] = nil
end
redirect(elem)
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_parameter_alias_after_rebind() {
    assert_primitive_call_boundary(
        r#"
local function redirect(receiver)
    receiver = other
    local saved = receiver
    saved[key] = nil
end
redirect(elem)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_parameter_alias_conditional_rebind() {
    assert_primitive_call_boundary(
        r#"
local function redirect(receiver)
    local saved = receiver
    if flag then saved = other end
    saved[key] = nil
end
redirect(elem)
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_parameter_alias_definite_rebind() {
    assert_primitive_call_boundary(
        r#"
local function redirect(receiver)
    local saved = receiver
    saved = other
    saved[key] = nil
end
redirect(elem)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_conditional_callee_rebind() {
    assert_primitive_call_boundary(
        r#"
local function writer(receiver) receiver[key] = nil end
local function readonly(receiver) return receiver end
local action = writer
if flag then action = readonly end
action(elem)
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_definite_callee_rebind() {
    assert_primitive_call_boundary(
        r#"
local function writer(receiver) receiver[key] = nil end
local function readonly(receiver) return receiver end
local action = writer
action = readonly
action(elem)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_later_callee_rebind() {
    assert_primitive_call_boundary(
        r#"
local function writer(receiver) receiver[key] = nil end
local function readonly(receiver) return receiver end
local action = writer
action(elem)
local value = elem.handler.name -- observe
action = readonly
"#,
        true,
    );
}

#[test]
fn long_aliases_and_distant_rebindings_keep_receiver_identity() {
    for (receiver, nullable) in [("elem", true), ("other", false)] {
        let mut aliases =
            format!("local function alias0() if flag then {receiver}[key] = nil end end\n");
        for alias in 1..40 {
            aliases.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
        }
        aliases.push_str("alias39()\nlocal value = elem.handler.name -- observe");
        assert_primitive_call_boundary(&aliases, nullable);

        let mut aliases =
            "local function redirect(target)\nlocal alias0 = target\ntarget = {}\n".to_string();
        for alias in 1..40 {
            aliases.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
        }
        aliases.push_str(&format!("if flag then alias39[key] = nil end\nend\nredirect({receiver})\nlocal value = elem.handler.name -- observe"));
        assert_primitive_call_boundary(&aliases, nullable);

        let padding = "count = count + 1\n".repeat(300);
        let body = format!(
            "local function writer() if flag then {receiver}[key] = nil end end\nlocal action = function() end\naction = writer\nlocal count = 0\n{padding}action()\nlocal value = elem.handler.name -- observe"
        );
        assert_primitive_call_boundary(&body, nullable);

        let original_receiver = if nullable { "other" } else { "elem" };
        let body = format!(
            "local function redirect(target)\ntarget = {receiver}\nlocal count = 0\n{padding}if flag then target[key] = nil end\nend\nredirect({original_receiver})\nlocal value = elem.handler.name -- observe"
        );
        assert_primitive_call_boundary(&body, nullable);
    }
}

#[test]
fn member_alias_values_use_the_assigning_callers_bindings() {
    for (first, second, nullable) in [
        ("target[key] = nil", "", true),
        ("", "target[key] = nil", false),
    ] {
        assert_member_call_boundary(
            "type(elem.handler) == 'table'",
            &format!(
                "local first = {{}}; local second = {{}}; function first.writer(target) {first} end; function second.writer(target) {second} end; local function run(m, target, next) if next then first.clear = m.writer; run(second, next, nil) else first.clear(target) end end"
            ),
            "run(first, other, elem)\nlocal value = elem.handler.name -- observe",
            nullable,
        );
    }
}

#[test]
fn saved_member_callees_include_alias_valued_table_fields() {
    for definition in [
        "local methods = {clear = writer}",
        "local methods = {clear = (writer)}",
        "local methods = {}; methods.clear = (writer)",
    ] {
        for (receiver, nullable) in [("elem", true), ("other", false)] {
            assert_member_call_boundary(
                "type(elem.handler) == 'table'",
                &format!(
                    "local function writer(target) target[key] = nil end; {definition}; local saved = methods.clear"
                ),
                &format!("saved({receiver})\nlocal value = elem.handler.name -- observe"),
                nullable,
            );
        }
    }
}

#[cfg(test)]
mod call_guard_followups {
    use super::*;
    use crate::LuaType;

    #[test]
    fn known_rawset_alias_invalidates_the_matching_receiver() {
        assert_primitive_call_boundary(
            "local write = rawset\nwrite(elem, 'handler', nil)\nlocal value = elem.handler.name -- observe",
            true,
        );
    }

    #[test]
    fn known_numeric_rawset_keys_preserve_a_named_field_guard() {
        for key in ["1.0", "1.5", "-1.5", "1e20"] {
            assert_primitive_call_boundary(
                &format!("rawset(elem, {key}, nil)\nlocal value = elem.handler.name -- observe"),
                false,
            );
        }
    }

    fn conditional_handler_type(statement: &str) -> LuaType {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            "---@class BranchHandler\n---@field name string\nlocal Handler = {{}}\n---@class BranchReceiver\n---@field handler BranchHandler|fun()|nil\nlocal Receiver = {{}}\n---@param elem BranchReceiver\n---@param other BranchReceiver\n---@param flag boolean\nlocal function dispatch(elem, other, flag)\nif type(elem.handler) == 'table' then\nif flag then {statement} end\nlocal value = elem.handler -- observe\nend\nend"
        );
        let marker = source.find("-- observe").unwrap();
        let start = source[..marker].rfind('\n').unwrap() + 1;
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let field = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|field| {
                field.get_text() == "elem.handler"
                    && (start..marker).contains(&(u32::from(field.get_position()) as usize))
            })
            .unwrap();
        model.type_of_expr_at(field.get_syntax_id(), field.get_position())
    }

    #[test]
    fn conditional_member_function_write_matches_assignment() {
        let assignment = conditional_handler_type("elem.handler = function() end");
        let definition = conditional_handler_type("function elem.handler() end");
        assert_eq!(definition, assignment);
    }
    #[test]
    fn rawset_aliases_follow_possible_reaching_values() {
        for (setup, nullable) in [
            (
                "local first = rawset; local write = (first); write(elem, 'handler', nil)",
                true,
            ),
            ("local write = rawset; write(other, 'handler', nil)", false),
            ("local write = rawset; write(elem, 'other', nil)", false),
            (
                "local write = rawset; write = function() end; write(elem, 'handler', nil)",
                false,
            ),
            (
                "local write = rawset; function write() end; write(elem, 'handler', nil)",
                false,
            ),
            (
                "local write = rawset; if flag then write = function() end end; write(elem, 'handler', nil)",
                true,
            ),
            (
                "local write = function() end; if flag then write = rawset end; write(elem, 'handler', nil)",
                true,
            ),
            (
                "local write = rawset; if flag then write = function() end else write = function() end end; write(elem, 'handler', nil)",
                false,
            ),
            (
                "local function rawset() end; local write = rawset; write(elem, 'handler', nil)",
                false,
            ),
            (
                "local first = rawset; local write = first; first = function() end; write(elem, 'handler', nil)",
                true,
            ),
            (
                "local write; write = write; write(elem, 'handler', nil)",
                false,
            ),
            (
                "local methods = {}; function methods.clear(target) target[key] = nil end; local set = rawset; set(methods, 'clear', function() end); methods.clear(elem)",
                false,
            ),
            (
                "local methods = {}; function methods.clear(target) target[key] = nil end; local set = rawset; set(other, 'clear', function() end); methods.clear(elem)",
                true,
            ),
        ] {
            assert_primitive_call_boundary(
                &format!("{setup}\nlocal value = elem.handler.name -- observe"),
                nullable,
            );
        }
    }

    #[test]
    fn rawset_aliases_preserve_truthy_and_primitive_guard_identity() {
        for guard in ["elem.handler", "type(elem.handler) == 'table'"] {
            for (receiver, nullable) in [("elem", true), ("other", false)] {
                assert_member_call_boundary(
                    guard,
                    "local write = rawset",
                    &format!(
                        "write({receiver}, 'handler', nil)\nlocal value = elem.handler.name -- observe"
                    ),
                    nullable,
                );
            }
        }
    }
}
