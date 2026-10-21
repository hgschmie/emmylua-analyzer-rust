use crate::{DiagnosticCode, VirtualWorkspace};

#[test]
fn standard_pairs_skips_nullable_values() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@param first string[]?
        ---@param second string[]?
        local function visit(first, second)
            for _, group in pairs {first, second} do
                for _, value in pairs(group) do print(value) end
            end
        end
    "#
    ));
}

#[test]
fn standard_pairs_removes_nil_from_nested_union_members() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@class Provider
        ---@field groups string[]
        local Provider = {}
        ---@class Train
        ---@field groups string[]?
        local Train = {}
        ---@param provider Provider
        ---@param train Train
        local function visit(provider, train)
            for _, group in pairs {provider.groups, train.groups} do
                for _, value in pairs(group) do print(value) end
            end
        end
    "#
    ));
}

#[test]
fn standard_pairs_preserves_false_values() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@param value string
        local function consume(value) end
        ---@param values table<string, string|false|nil>
        local function visit(values)
            for _, value in pairs(values) do consume(value) end
        end
    "#
    ));
}

#[test]
fn custom_iterator_values_can_still_be_nil() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@return fun(): integer?, string?
        local function iterate() end
        ---@param value string
        local function consume(value) end
        for _, value in iterate() do consume(value) end
    "#
    ));
}

#[test]
fn shadowed_pairs_retains_its_nullable_value_contract() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@generic K, V
        ---@param values table<K,V>
        ---@return fun(): K, V?
        local function pairs(values) end
        ---@param value string
        local function consume(value) end
        ---@param values table<string,string>
        local function visit(values)
            for _, value in pairs(values) do consume(value) end
        end
    "#
    ));
}

#[test]
fn nullable_collection_argument_still_requires_a_guard() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@param values string[]?
        local function visit(values)
            for _, value in pairs(values) do print(value) end
        end
    "#
    ));
}
