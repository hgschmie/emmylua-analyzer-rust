use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

#[test]
fn concrete_array_arguments_preserve_declared_returns() {
    for argument in ["{}", "{{}}", "{{name = 'fuel'}}"] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            ---@param filters table[]
            ---@return string
            function select_fuel(filters) end
        "#,
        );
        assert_eq!(
            ws.expr_ty(&format!("select_fuel({argument})")),
            LuaType::String
        );
    }
}

#[test]
fn nested_structural_arrays_preserve_declared_returns() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@class Filter
        ---@field name string
        local Filter = {}
        ---@param filters Filter[]
        ---@return integer
        function select_fuel(filters) end
    "#,
    );
    assert_eq!(
        ws.expr_ty("select_fuel({{name = 'fuel'}})"),
        LuaType::Integer
    );
}

#[test]
fn table_argument_to_named_union_preserves_return_alias() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@alias Key string
        ---@class Signal
        ---@field name string
        local Signal = {}
        ---@class Item
        ---@field count integer
        local Item = {}
        ---@param value Signal|Item
        ---@return Key
        function make_key(value) end
    "#,
    );
    let expected = ws.ty("Key");
    assert_eq!(ws.expr_ty("make_key({name = 'fuel'})"), expected);
}

#[test]
fn table_call_assignment_initializes_lazy_cache() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ReturnTypeMismatch,
        r#"
        ---@class Cache<K, V>: table<K,V>
        local Cache = {}
        local prototypes = {
            ---@param filters table[]
            ---@return Cache<string, integer>
            get_filtered = function(filters) end,
        }
        ---@type Cache<string, integer>?
        local cache
        ---@return Cache<string, integer>
        local function get()
            if not cache then cache = prototypes.get_filtered {{filter = 'fuel'}} end
            return cache
        end
    "#
    ));
}

#[test]
fn incompatible_literal_fields_and_nullable_arguments_are_still_diagnosed() {
    for value in ["nil", "42"] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(
            !ws.has_no_diagnostic(
                DiagnosticCode::ParamTypeMismatch,
                &format!(
                    r#"
            ---@param value {{name: string}}|{{count: integer}}
            ---@return string
            local function make_key(value) end
            make_key({value})
        "#
                )
            ),
            "{value}"
        );
    }
}

#[test]
fn structural_overloads_select_the_matching_return() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@overload fun(value: {name: string}): string
        ---@param value {count: integer}
        ---@return integer
        function select_value(value) end
    "#,
    );
    assert_eq!(ws.expr_ty("select_value({name = 'fuel'})"), LuaType::String);
    assert_eq!(ws.expr_ty("select_value({count = 2})"), LuaType::Integer);
}

#[test]
fn incompatible_structural_argument_does_not_select_an_overload() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@param value {name: string}|{count: integer}
        ---@return string
        function make_key(value) end
    "#,
    );
    assert_eq!(ws.expr_ty("make_key({name = 42})"), LuaType::Unknown);
}
