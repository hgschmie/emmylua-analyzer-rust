use crate::{DiagnosticCode, VirtualWorkspace};
use crate::{LuaType, VariadicType};
const DEFS: &str = r#"
---@class KnownInterface
---@field send fun(name:string, flag:boolean): integer
---@class (partial) Interfaces
---@field known KnownInterface
---@alias Any string|boolean|number|table
---@generic I: (keyof Interfaces)|string, F: I extends keyof Interfaces and keyof Interfaces[I] or string, FF: I extends keyof Interfaces and Interfaces[I][F] or (fun(...:Any?):Any?...)
---@param interface I
---@param fname F
---@param ... Parameters<FF>...
---@return ReturnType<FF>...
function invoke(interface,fname,...) end
"#;

fn workspace() -> VirtualWorkspace {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(DEFS);
    ws
}

#[test]
fn dependent_call_unknown_interface_fallback() {
    let mut ws = workspace();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "invoke('other', 'send', 'hello', true, nil, {})"
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "invoke('other', 'send', function() end)"
    ));
}

#[test]
fn dependent_call_declared_interface_parameters() {
    let mut ws = workspace();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "invoke('known', 'send', 'hello', true)"
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "invoke('known', 'send', false, 42)"
    ));
}

#[test]
fn dependent_call_declared_interface_method_constraint() {
    let mut ws = workspace();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "invoke('known', 'missing', 'hello', true)"
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::GenericConstraintMismatch,
        "invoke('known', 'missing')"
    ));
}

#[test]
fn dependent_call_declared_return_type() {
    let mut ws = workspace();
    assert_eq!(
        ws.expr_ty("invoke('known', 'send', 'hello', true)"),
        LuaType::Integer
    );
}

#[test]
fn dependent_call_fallback_return_type() {
    let mut ws = workspace();
    let ty = ws.expr_ty("invoke('unknown', 'send', 'hello', true)");
    let LuaType::Variadic(variadic) = ty else {
        panic!("{ty:?}")
    };
    let VariadicType::Base(base) = variadic.as_ref() else {
        panic!("{variadic:?}")
    };
    assert_eq!(base, &ws.ty("Any?"));
}

#[test]
fn dependent_call_preserves_structural_generic_inference() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
---@class Animal
---@class Dog: Animal
---@field bark fun()
---@generic T: Animal
---@param values T[]
---@return T
function first(values) end
---@type Dog[]
dogs = {}
"#,
    );
    assert_eq!(ws.expr_ty("first(dogs)"), ws.ty("Dog"));
    assert!(ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, "first(dogs)"));
}

#[test]
fn dependent_call_cross_file_dot_member_and_hyphenated_interface() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def("remote = {}");
    ws.def(
        &DEFS
            .replace("known KnownInterface", "['known-mod'] KnownInterface")
            .replace(
                "function invoke(interface,fname,...) end",
                "function remote.call(interface,fname,...) end",
            ),
    );
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "remote.call('known-mod', 'send', 'hello', true)"
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "remote.call('known-mod', 'send', 42, false)"
    ));
    assert_eq!(
        ws.expr_ty("remote.call('known-mod', 'send', 'hello', true)"),
        LuaType::Integer
    );
}

#[test]
fn dependent_call_multiple_and_optional_returns() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(&DEFS.replace(
        "---@class KnownInterface",
        "---@class KnownInterface\n---@field pair fun(): string, integer?",
    ));
    let ty = ws.expr_ty("invoke('known', 'pair')");
    let LuaType::Variadic(variadic) = ty else {
        panic!("{ty:?}")
    };
    let VariadicType::Multi(types) = variadic.as_ref() else {
        panic!("{variadic:?}")
    };
    assert_eq!(types, &[LuaType::String, ws.ty("integer?")]);
}

#[test]
fn dependent_call_default_order_and_explicit_binding() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
---@generic T = string, U = T[]
---@return U
function defaults() end
---@generic T: number
---@param value T
---@return T
function identity(value) end
"#,
    );
    assert_eq!(ws.expr_ty("defaults()"), ws.ty("string[]"));
    assert_eq!(ws.expr_ty("identity--[[@<number>]](1)"), LuaType::Number);
}

#[test]
fn dependent_call_constraints_accept_valid_methods() {
    let mut ws = workspace();
    for code in [
        DiagnosticCode::ParamTypeMismatch,
        DiagnosticCode::GenericConstraintMismatch,
    ] {
        assert!(ws.has_no_diagnostic(
            code,
            "invoke('other', 'send', 'hello', true)\ninvoke('known', 'send', 'hello', true)"
        ));
    }
    assert_eq!(
        ws.expr_ty("invoke('known', 'send', 'hello', true)"),
        LuaType::Integer
    );
}

#[test]
fn dependent_call_constraints_reject_invalid_method_and_arguments() {
    let mut ws = workspace();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::GenericConstraintMismatch,
        "invoke('known', 'missing')"
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "invoke('known', 'send', false, 42)"
    ));
}

// Integration with UP-019: generated Factorio API classes use global table
// initializers, whose member ownership is not yet connected on upstream/main.
#[test]
fn dependent_call_generated_global_api_table() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let (defs, signature) = DEFS.split_once("---@generic").unwrap();
    ws.def(defs);
    ws.def(&format!(
        "---@meta _\ndo\n---@class Remote\nremote = {{\n---@generic{}\n}}\nend",
        signature.replace(
            "function invoke(interface,fname,...) end",
            "call = function(interface,fname,...) end"
        )
    ));
    for code in [
        DiagnosticCode::ParamTypeMismatch,
        DiagnosticCode::GenericConstraintMismatch,
    ] {
        assert!(ws.has_no_diagnostic(code,
            "remote.call('other', 'send', 'hello', true)\nremote.call('known', 'send', 'hello', true)"));
    }
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::GenericConstraintMismatch,
        "remote.call('known', 'missing')"
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        "remote.call('known', 'send', false, 42)"
    ));
    assert_eq!(
        ws.expr_ty("remote.call('known', 'send', 'hello', true)"),
        LuaType::Integer
    );
}
