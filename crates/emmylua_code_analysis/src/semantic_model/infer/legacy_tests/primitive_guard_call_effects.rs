use crate::{DiagnosticCode, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaIndexExpr};

#[test]
fn called_writer_guard_lifetime_conditional_nil_keeps_both_branches() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::NeedCheckNil);
    let source = r#"---@class GuardCallHandler
---@field name string
local Handler = {}
---@class GuardCallReceiver
---@field handler GuardCallHandler?
local Receiver = {}
---@param elem GuardCallReceiver
---@param flag boolean
---@param key string
local function dispatch(elem, flag, key)
    local function clear() if flag then elem[key] = nil end end
    if type(elem.handler) == 'table' then
        clear()
        local after = elem.handler -- after
        local name = elem.handler.name -- dereference
        if elem.handler then
            local yes = elem.handler -- yes
        else
            local no = elem.handler -- no
        end
    end
end
"#;
    let file = ws.def(source);
    let expected_optional = ws.ty("GuardCallHandler?");
    let expected_handler = ws.ty("GuardCallHandler");
    let diagnostics = ws
        .analysis
        .diagnose_file(file, tokio_util::sync::CancellationToken::new())
        .unwrap();
    let model = ws.analysis.semantic_model(file);
    let mut results = Vec::new();
    for marker in ["-- after", "-- dereference", "-- yes", "-- no"] {
        let end = source.find(marker).unwrap();
        let start = source[..end].rfind('\n').unwrap() + 1;
        let field = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|field| {
                field.get_text() == "elem.handler"
                    && (start..end).contains(&(u32::from(field.get_position()) as usize))
            })
            .unwrap();
        let typ = model.type_of_expr_at(field.get_syntax_id(), field.get_position());
        results.push(typ);
    }
    assert_eq!(
        &results[..3],
        &[
            expected_optional.clone(),
            expected_optional,
            expected_handler
        ]
    );
    assert!(
        results[3].is_nullable(),
        "false branch must retain nil: {:?}",
        results[3]
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line
                == source
                    .lines()
                    .position(|line| line.contains("-- dereference"))
                    .unwrap() as u32),
        "required conditional nil warning: {diagnostics:#?}"
    );
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::UnnecessaryIf);
    let diagnostics = ws
        .analysis
        .diagnose_file(file, tokio_util::sync::CancellationToken::new())
        .unwrap();
    let condition = source
        .lines()
        .position(|line| line.contains("if elem.handler then"))
        .unwrap() as u32;
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == condition),
        "conditional write must not make the next condition constant: {diagnostics:#?}"
    );
}

fn assert_primitive_call_boundary(body: &str, nullable: bool) {
    assert_member_call_boundary("type(elem.handler) == 'table'", "", body, nullable);
}

fn assert_member_call_boundary(guard: &str, setup: &str, body: &str, nullable: bool) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::NeedCheckNil);
    let source = format!(
        r#"
---@class CallBoundaryHandler
---@field name string
local Handler = {{}}
---@class CallBoundaryReceiver
---@field handler CallBoundaryHandler?
local Receiver = {{}}
---@param elem CallBoundaryReceiver
---@param key string
---@param flag boolean
---@param other CallBoundaryReceiver
local function dispatch(elem, key, flag, other)
    local function clear() if flag then elem[key] = nil end end
{setup}
    if {guard} then
{body}
    end
end
"#
    );
    let marker = source.find("-- observe").unwrap();
    let start = source[..marker].rfind('\n').unwrap() + 1;
    let expected = ws.ty(if nullable {
        "CallBoundaryHandler?"
    } else {
        "CallBoundaryHandler"
    });
    let file = ws.def(&source);
    let model = ws.analysis.semantic_model(file);
    let field = model
        .chunk()
        .unwrap()
        .descendants::<LuaIndexExpr>()
        .filter(|field| {
            field.get_text() == "elem.handler"
                && (start..marker).contains(&(u32::from(field.get_position()) as usize))
        })
        .max_by_key(LuaAstNode::get_position)
        .unwrap();
    let actual = model.type_of_expr_at(field.get_syntax_id(), field.get_position());
    assert_eq!(actual, expected, "{body}");
    assert_eq!(
        model.type_of_expr_at(field.get_syntax_id(), field.get_position()),
        expected,
        "warm query: {body}"
    );
    let diagnostics = ws
        .analysis
        .diagnose_file(file, tokio_util::sync::CancellationToken::new())
        .unwrap();
    let line = source[..marker].bytes().filter(|ch| *ch == b'\n').count() as u32;
    assert_eq!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == line),
        nullable,
        "{body}: {diagnostics:#?}"
    );
}

#[test]
fn primitive_guard_call_boundary_statement() {
    assert_primitive_call_boundary(
        r#"
clear()
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_local_initializer() {
    assert_primitive_call_boundary(
        r#"
local ignored = clear()
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_assignment() {
    assert_primitive_call_boundary(
        r#"
local ignored
ignored = clear()
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_condition() {
    assert_primitive_call_boundary(
        r#"
if clear() then end
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_earlier_argument() {
    assert_primitive_call_boundary(
        r#"
local value = select(1, clear(), elem.handler.name) -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_later_argument() {
    assert_primitive_call_boundary(
        r#"
local value = select(1, elem.handler.name, clear()) -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_uncalled_closure() {
    assert_primitive_call_boundary(
        r#"
local deferred = function() clear() end
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_pure_type() {
    assert_primitive_call_boundary(
        r#"
type(false)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_pure_scalar_tostring() {
    assert_primitive_call_boundary(
        r#"
local text = tostring(42)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_opaque_tostring_preserves_guard() {
    assert_primitive_call_boundary(
        r#"
local text = tostring(elem)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_fresh_guard() {
    assert_primitive_call_boundary(
        r#"
clear()
if type(elem.handler) == "table" then
 local value = elem.handler.name -- observe
end
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_conditional_call_join() {
    assert_primitive_call_boundary(
        r#"
if flag then clear() end
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_later_call() {
    assert_primitive_call_boundary(
        r#"
local value = elem.handler.name -- observe
clear()
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_returning_branch() {
    assert_primitive_call_boundary(
        r#"
if flag then clear(); return end
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_opaque_receiver_call_preserves_guard() {
    assert_primitive_call_boundary(
        r#"
opaque(elem)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_unrelated_receiver_write() {
    assert_primitive_call_boundary(
        r#"
other[key] = nil
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_same_receiver_write() {
    assert_primitive_call_boundary(
        r#"
elem[key] = nil
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_receiver_replacement() {
    assert_primitive_call_boundary(
        r#"
elem = other
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_readonly_local() {
    assert_primitive_call_boundary(
        r#"
local function readonly() local ignored = elem.handler end
readonly()
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_other_receiver_call() {
    assert_primitive_call_boundary(
        r#"
local function clear_other() other[key] = nil end
clear_other()
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_parameter_same_receiver() {
    assert_primitive_call_boundary(
        r#"
local function clear_receiver(receiver) receiver[key] = nil end
clear_receiver(elem)
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_parameter_other_receiver() {
    assert_primitive_call_boundary(
        r#"
local function clear_receiver(receiver) receiver[key] = nil end
clear_receiver(other)
local value = elem.handler.name -- observe
"#,
        false,
    );
}

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
fn primitive_guard_call_boundary_transitive_visible_write() {
    assert_primitive_call_boundary(
        r#"
local function wrapper() clear() end
wrapper()
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_inline_visible_write() {
    assert_primitive_call_boundary(
        r#"
(function() elem[key] = nil end)()
local value = elem.handler.name -- observe
"#,
        true,
    );
}

#[test]
fn primitive_guard_call_boundary_known_other_field_write() {
    assert_primitive_call_boundary(
        r#"
local function clear_other_field() elem.other = nil end
clear_other_field()
local value = elem.handler.name -- observe
"#,
        false,
    );
}

#[test]
fn primitive_guard_call_boundary_uncalled_child_writer() {
    assert_primitive_call_boundary(
        r#"
local function wrapper() local deferred = function() elem[key] = nil end end
wrapper()
local value = elem.handler.name -- observe
"#,
        false,
    );
}

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
fn primitive_guard_call_boundary_distinct_parameter_bindings() {
    assert_primitive_call_boundary(
        r#"
local function mutate(target) target[key] = nil end
mutate(other)
mutate(elem)
local value = elem.handler.name -- observe
"#,
        true,
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
fn primitive_guard_index_fallback_respects_visible_call_effects() {
    for (body, nullable) in [
        (
            "clear(); if flag then else print(elem.handler.name) -- observe\nend",
            true,
        ),
        (
            "local changed = clear(); if flag then else print(elem.handler.name) -- observe\nend",
            true,
        ),
        (
            "if clear() then else print(elem.handler.name) -- observe\nend",
            true,
        ),
        (
            "if flag then else print(clear(), elem.handler.name) -- observe\nend",
            true,
        ),
        (
            "opaque(elem); if flag then else print(elem.handler.name) -- observe\nend",
            false,
        ),
        (
            "local function change() other.handler = nil end; change(); if flag then else print(elem.handler.name) -- observe\nend",
            false,
        ),
        (
            "if flag then else print(elem.handler.name, clear()) -- observe\nend",
            false,
        ),
        (
            "clear(); if type(elem.handler) == 'table' then if flag then else\nprint(elem.handler.name) -- observe\nend end",
            false,
        ),
    ] {
        assert_primitive_call_boundary(body, nullable);
    }
}

#[test]
fn visible_calls_invalidate_truthiness_and_nil_guards() {
    for guard in [
        "elem.handler",
        "elem.handler ~= nil",
        "not (elem.handler == nil)",
        "type(elem.handler) == 'table'",
    ] {
        for (body, nullable) in [
            ("clear()\nlocal value = elem.handler.name -- observe", true),
            (
                "opaque(elem)\nlocal value = elem.handler.name -- observe",
                false,
            ),
            (
                "clear()\nif elem.handler then\nlocal value = elem.handler.name -- observe\nend",
                false,
            ),
        ] {
            assert_member_call_boundary(guard, "", body, nullable);
        }
    }
}

#[test]
fn member_guards_respect_call_order_in_conditions() {
    for (guard, nullable) in [
        ("check() and type(elem.handler) == 'table'", false),
        ("type(elem.handler) == 'table' and check()", true),
        ("(check() or flag) and type(elem.handler) == 'table'", false),
        ("type(elem.handler) == 'table' and opaque(elem)", false),
    ] {
        assert_member_call_boundary(
            guard,
            "local function check() clear(); return flag end",
            "local value = elem.handler.name -- observe",
            nullable,
        );
    }
}

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
fn inline_member_guards_follow_preceding_calls() {
    for (body, nullable) in [
        (
            "local value = check() and type(elem.handler) == 'table' and elem.handler.name -- observe",
            false,
        ),
        (
            "local value = type(elem.handler) == 'table' and check() and elem.handler.name -- observe",
            true,
        ),
        (
            "local value = check() and elem.handler and elem.handler.name -- observe",
            false,
        ),
        (
            "local value = elem.handler and check() and elem.handler.name -- observe",
            true,
        ),
    ] {
        assert_member_call_boundary(
            "flag",
            "local function check() clear(); return flag end",
            body,
            nullable,
        );
    }
}

#[test]
fn visible_calls_invalidate_falsy_member_guards() {
    for guard in [
        "not elem.handler",
        "elem.handler == nil",
        "not (elem.handler ~= nil)",
    ] {
        assert_member_call_boundary(
            guard,
            "",
            "clear()\nlocal value = elem.handler.name -- observe",
            true,
        );
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
