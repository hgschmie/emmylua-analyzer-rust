use crate::{DiagnosticCode, VirtualWorkspace};

const DEFS: &str = r#"
---@enum Event
local Events = { tick = 1, stopped = 2 }
---@enum OtherEvent
local OtherEvents = { changed = 3 }
---@class Base
---@field name string
---@class Child: Base
---@field count integer
---@class Unrelated
---@field enabled boolean
---@alias EventAlias Event
---@alias ChildAlias Child
---@return Event...
function events() end
---@return Child...
function children() end
---@return ChildAlias...
function aliased_children() end
---@return Unrelated...
function unrelated() end
---@param event Event
---@param handler function
function register(event, handler) end
---@param event OtherEvent
---@param handler function
function register_other(event, handler) end
---@param value Base
---@param handler function
function consume(value, handler) end
---@param event EventAlias
---@param handler function
function register_alias(event, handler) end
---@param event Event|string
---@param handler function
function register_union(event, handler) end
"#;

fn accepts(code: &str) -> bool {
    let mut ws = VirtualWorkspace::new();
    ws.def(DEFS);
    ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, code)
}

#[test]
fn variadic_named_enum_arguments() {
    assert!(accepts("register(events(), function() end)"));
    assert!(accepts("register((events()), function() end)"));
}

#[test]
fn variadic_named_inherited_class_arguments() {
    assert!(accepts("consume(children(), function() end)"));
}

#[test]
fn variadic_named_alias_arguments() {
    assert!(accepts("register_alias(events(), function() end)"));
    assert!(accepts("consume(aliased_children(), function() end)"));
}

#[test]
fn variadic_named_explicit_union_arguments() {
    assert!(accepts("register_union(events(), function() end)"));
}

#[test]
fn variadic_named_arguments_still_reject_unrelated_types() {
    assert!(!accepts("register_other(events(), function() end)"));
    assert!(!accepts("consume(unrelated(), function() end)"));
    assert!(!accepts("register(children(), function() end)"));
}

#[test]
fn variadic_named_overloaded_arguments() {
    assert!(accepts(
        r#"
---@param event Event
---@param handler function
---@overload fun(event:string, handler:function)
function overloaded(event, handler) end
overloaded(events(), function() end)
"#
    ));
}

#[test]
fn variadic_named_local_and_tuple_controls() {
    assert!(accepts(
        r#"
local event = events()
register(event, function() end)
---@return Event, string
local function tuple_event() end
register(tuple_event(), function() end)
"#
    ));
}

#[test]
fn variadic_named_elements_match_fixed_target_slots() {
    use crate::{LuaType, VariadicType};
    use std::rc::Rc;

    let mut ws = VirtualWorkspace::new();
    ws.def(DEFS);
    let event = ws.ty("Event");
    let other = ws.ty("OtherEvent");
    let source = LuaType::Variadic(Rc::new(VariadicType::Base(event.clone())));
    let target = LuaType::Variadic(Rc::new(VariadicType::Multi(vec![
        event.clone(),
        event.clone(),
    ])));
    assert!(ws.check_type(&source, &target));
    let invalid = LuaType::Variadic(Rc::new(VariadicType::Multi(vec![event, other])));
    assert!(!ws.check_type(&source, &invalid));
}

#[test]
fn variadic_single_target_uses_result_compatibility() {
    use crate::{LuaType, VariadicType};
    use std::rc::Rc;

    let mut ws = VirtualWorkspace::new();
    ws.def(DEFS);
    let child = ws.ty("Child?");
    let base = ws.ty("Base");
    let mixed_event = ws.ty("Event|OtherEvent");
    let event = ws.ty("Event");
    for repeat in [false, true] {
        let variadic = |ty| {
            LuaType::Variadic(Rc::new(if repeat {
                VariadicType::Repeat(ty)
            } else {
                VariadicType::Base(ty)
            }))
        };
        assert!(ws.check_type(&variadic(LuaType::Integer), &LuaType::Number));
        assert!(!ws.check_type(&variadic(LuaType::Number), &LuaType::Integer));
        assert!(!ws.check_type(&variadic(child.clone()), &base));
        assert!(!ws.check_type(&variadic(mixed_event.clone()), &event));
    }
}
