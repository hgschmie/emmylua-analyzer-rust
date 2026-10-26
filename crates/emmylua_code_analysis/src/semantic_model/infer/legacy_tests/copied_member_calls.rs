use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

fn assert_types(source: &str, expected: &[(&str, LuaType)]) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    for (name, expected) in expected {
        let decl = model
            .decls()
            .unwrap()
            .iter()
            .find(|decl| decl.name == *name)
            .unwrap();
        assert_eq!(
            model.type_of_decl(&decl.id),
            Some(expected.clone()),
            "{name}: {source}"
        );
    }
}

#[test]
fn copied_member_call_preserves_signature_identity() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def("local alias = math.ceil\nlocal result = alias(1.5)");
    let model = ws.analysis.semantic_model(file);
    let decls = model.decls().unwrap();
    let alias = decls.iter().find(|decl| decl.name == "alias").unwrap();
    assert!(matches!(
        model.type_of_decl(&alias.id),
        Some(LuaType::Signature(_))
    ));
    let result = decls.iter().find(|decl| decl.name == "result").unwrap();
    assert_eq!(model.type_of_decl(&result.id), Some(LuaType::Integer));
}

#[test]
fn copied_member_calls_preserve_captured_and_chained_returns() {
    for aliases in [
        "local alias = math.ceil",
        "local first = math.ceil\nlocal alias = first",
    ] {
        assert_types(
            &format!(
                "{aliases}\n---@param value number\nlocal function run(value)\nlocal result = alias(value)\nreturn result\nend"
            ),
            &[("result", LuaType::Integer)],
        );
    }
}

#[test]
fn copied_member_calls_preserve_custom_multi_returns() {
    assert_types(
        r#"
        local lib = {}
        ---@return integer, string
        function lib.round() return 1, 'done' end
        local alias = lib.round
        local first, second = alias()
    "#,
        &[("first", LuaType::Integer), ("second", LuaType::String)],
    );
}

#[test]
fn copied_member_calls_preserve_overloads() {
    assert_types(
        r#"
        local lib = {}
        ---@param value number
        ---@return integer
        ---@overload fun(value: string): string
        function lib.convert(value) return 1 end
        local alias = lib.convert
        local first = alias(1.5)
        local second = alias('text')
    "#,
        &[("first", LuaType::Integer), ("second", LuaType::String)],
    );
}

#[test]
fn copied_member_calls_preserve_generics() {
    assert_types(
        r#"
        local lib = {}
        ---@generic T
        ---@param value T
        ---@return T
        function lib.identity(value) return value end
        local alias = lib.identity
        ---@param value string
        local function run(value)
            local result = alias(value)
            return result
        end
    "#,
        &[("result", LuaType::String)],
    );
}

#[test]
fn copied_member_calls_keep_copied_value_after_member_replacement() {
    assert_types(
        r#"
        local lib = {}
        ---@return integer
        function lib.round() return 1 end
        local alias = lib.round
        lib.round = function() return 'later' end
        local result = alias()
    "#,
        &[("result", LuaType::Integer)],
    );
}

#[test]
fn copied_member_calls_use_reassigned_local_value() {
    assert_types(
        r#"
        local alias = math.ceil
        ---@return string
        local function later() return 'later' end
        alias = later
        local result = alias()
    "#,
        &[("result", LuaType::String)],
    );
}

#[test]
fn copied_member_calls_respect_explicit_alias_type() {
    assert_types(
        r#"
        ---@type fun(value: number): string
        local alias = math.ceil
        local result = alias(1.5)
    "#,
        &[("result", LuaType::String)],
    );
}

#[test]
fn copied_member_calls_keep_invalid_arguments() {
    for callee in ["math.ceil", "alias"] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            &format!("local alias = math.ceil\nlocal result = {callee}('invalid')")
        ));
    }
}

#[test]
fn copied_member_calls_preserve_cross_file_returns() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def_file(
        "lib.lua",
        "local lib = {}\n---@return integer\nfunction lib.round() return 1 end\nreturn lib",
    );
    let file =
        ws.def("local lib = require('lib')\nlocal alias = lib.round\nlocal result = alias()");
    let model = ws.analysis.semantic_model(file);
    let result = model
        .decls()
        .unwrap()
        .iter()
        .find(|decl| decl.name == "result")
        .unwrap();
    assert_eq!(model.type_of_decl(&result.id), Some(LuaType::Integer));
}

#[test]
fn copied_member_calls_preserve_return_overload_slots() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        local lib = {}
        ---@generic T, R
        ---@param f fun(...: T...): R...
        ---@param ... T...
        ---@return_overload true, R...
        ---@return_overload false, string
        function lib.wrap(f, ...) return true, f(...) end
        local alias = lib.wrap
        ---@param value integer
        ---@return integer, string, boolean
        local function produce(value) return value, tostring(value), true end
        ok, first, second, third = alias(produce, 1)
    "#,
    );
    for (name, expected) in [
        ("ok", "false|true"),
        ("first", "integer|string"),
        ("second", "string|nil"),
        ("third", "boolean|nil"),
    ] {
        assert_eq!(ws.expr_ty(name), ws.ty(expected), "{name}");
    }
}

#[test]
fn copied_member_calls_preserve_declared_member_contracts() {
    assert_types(
        r#"
        ---@class CopyService
        ---@field get fun(): string
        ---@param service CopyService
        local function run(service)
            local alias = service.get
            local result = alias()
            return result
        end
    "#,
        &[("result", LuaType::String)],
    );
    assert_types(
        r#"
        ---@class CopyBox<T>
        ---@field get fun(): T
        ---@param box CopyBox<string>
        local function run(box)
            local alias = box.get
            local result = alias()
            return result
        end
    "#,
        &[("result", LuaType::String)],
    );
}

#[test]
fn copied_member_calls_keep_explicit_broad_member_contract() {
    assert_types(
        r#"
        ---@class CopyBroadService
        ---@field get function
        local Service = {}
        ---@return integer
        function Service.get() return 1 end
        local alias = Service.get
        local result = alias()
    "#,
        &[("result", LuaType::Unknown)],
    );
}
