use crate::{DiagnosticCode, VirtualWorkspace};
use lsp_types::NumberOrString;
use tokio_util::sync::CancellationToken;

const TYPES: &str = r#"
---@class Connector
---@field id integer
---@class Box<T>
---@field value T
---@alias Required Connector
---@alias Optional Connector?
---@alias OptionalChain Optional
---@alias Maybe<T> T|nil
---@alias IndirectMaybe Maybe<Connector>
"#;

fn check(source: &str, expected: usize) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::ParamTypeMismatch);
    ws.def_file("types.lua", TYPES);
    let file = ws.def(source);
    let diagnostics = ws
        .analysis
        .diagnose_file(file, CancellationToken::new())
        .unwrap();
    assert_eq!(diagnostics.len(), expected, "{source}\n{diagnostics:#?}");
    for diagnostic in diagnostics {
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("param-type-mismatch".into()))
        );
    }
}

#[test]
fn nullable_required_arguments_reject_named_generic_and_alias_values() {
    for (parameter, argument) in [
        ("Connector", "Connector?"),
        ("Required", "OptionalChain"),
        ("Connector", "Maybe<Connector>"),
        ("Connector", "IndirectMaybe"),
        ("Connector|string", "Connector?"),
        ("Box<integer>", "Box<integer>?"),
    ] {
        check(
            &format!(
                r#"
---@param target {parameter}
local function required(target) end
---@param value {argument}
local function use(value) required(value) end
"#
            ),
            1,
        );
    }
}

#[test]
fn nullable_required_arguments_check_dot_colon_and_pcall() {
    check(
        r#"
local api = {
    ---@param target Connector
    connect = function(target) end,
}
---@param target Connector
function api:connect_method(target) end
---@param target Connector?
local function use(target)
    api.connect(target)
    api:connect_method(target)
    pcall(api.connect, target)
end
"#,
        3,
    );
}

#[test]
fn nullable_required_arguments_check_variadic_slots() {
    for parameter in ["Connector", "Connector..."] {
        check(
            &format!(
                r#"
---@param ... {parameter}
local function connect(...) end
---@param target Connector?
local function use(target) connect(target, target) end
"#
            ),
            2,
        );
    }
}

#[test]
fn nullable_required_arguments_keep_overload_contracts_separate() {
    check(
        r#"
---@type fun(target: Connector)|fun(target: string)
local required
---@type fun(target: Connector)|fun(target: Connector?)
local optional
---@param target Connector?
local function use(target)
    required(target)
    optional(target)
end
"#,
        1,
    );
}

#[test]
fn nullable_required_arguments_preserve_optional_and_unknown_contracts() {
    for (parameter, argument) in [
        ("Connector?", "Connector?"),
        ("OptionalChain", "Connector?"),
        ("Maybe<Connector>", "Connector?"),
        ("Connector|string|nil", "Connector?"),
        ("Box<integer>?", "Box<integer>?"),
        ("any", "Connector?"),
        ("unknown", "Connector?"),
        ("Connector", "any"),
        ("Connector", "unknown"),
    ] {
        check(
            &format!(
                r#"
---@param target {parameter}
local function accept(target) end
---@param value {argument}
local function use(value) accept(value) end
"#
            ),
            0,
        );
    }
}

#[test]
fn nullable_required_arguments_respect_flow_guards() {
    check(
        r#"
---@param target Connector
local function required(target) end
---@param target Connector?
local function use(target)
    if target then required(target) end
    required(assert(target))
end
---@param target Connector?
local function asserted(target)
    assert(target)
    required(target)
end
---@param target Connector?
local function returned(target)
    if not target then return end
    required(target)
end
"#,
        0,
    );
}

#[test]
fn nullable_required_arguments_allow_inferred_nullable_generics() {
    check(
        r#"
---@generic T
---@param target T
local function accept(target) end
---@generic T
---@param ... T...
local function sequence(...) end
---@param target Connector?
local function use(target)
    accept(target)
    sequence(target, nil)
end
"#,
        0,
    );
}

#[test]
fn nullable_required_arguments_reject_explicit_required_generics() {
    check(
        r#"
---@generic T
---@param target T
local function required(target) end
---@param target Connector?
local function use(target)
    required--[[@<Connector>]](target)
end
"#,
        1,
    );
}

#[test]
fn nullable_required_arguments_check_indexed_values() {
    check(
        r#"
---@param target Connector
local function required(target) end
---@param connectors Connector[]
local function use(connectors)
    required(connectors[1])
    local connector = connectors[1]
    if connector then required(connector) end
end
"#,
        1,
    );
}

#[test]
fn nullable_required_arguments_respect_member_guards_and_casts() {
    check(
        r#"
---@param target Connector
local function required(target) end
---@param connectors Connector[]
local function asserted(connectors)
    assert(connectors[1])
    required(connectors[1])
end
---@param connectors Connector[]
local function casted(connectors)
    required(connectors[1] --[[@as Connector]])
end
"#,
        0,
    );
}

#[test]
fn nullable_required_arguments_do_not_reuse_guards_after_writes() {
    check(
        r#"
---@param target Connector
local function required(target) end
---@param connectors Connector[]
local function changed(connectors)
    assert(connectors[1])
    connectors[1] = nil
    required(connectors[1])
end
"#,
        1,
    );
}
