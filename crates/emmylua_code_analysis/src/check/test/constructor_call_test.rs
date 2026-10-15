use super::check_source;
use crate::DiagnosticCode;

#[test]
fn constructor_call_aligns_colon_and_dot_receivers() {
    for declaration in [
        "---@field put fun(self: Api, options: Options)",
        "---@field put fun(receiver: Api, options: Options)",
    ] {
        for call in ["api:put {count = 1}", "api.put(api, {count = 1})"] {
            let diagnostics = check_source(&format!(
                r#"
                ---@class Options
                ---@field count integer
                ---@class Api
                ---@field name string
                {declaration}
                ---@type Api
                local api
                {call}
            "#
            ));
            assert!(
                diagnostics.is_empty(),
                "{declaration}; {call}: {diagnostics:?}"
            );
        }
    }
}

#[test]
fn constructor_call_only_checks_the_matching_overload() {
    let diagnostics = check_source(
        r#"
        ---@class Item
        ---@field name string
        ---@class Setting
        ---@field enabled boolean
        ---@type Item
        local item
        ---@type Setting
        local setting
        ---@class Api
        ---@field raw table
        ---@field is_demo boolean
        ---@field extend fun(self: Api, items: Item[])
        ---@field extend fun(self: Api, items: Setting[])
        ---@type Api
        local api
        api:extend {item}
        api:extend {setting}
        api.extend(api, {item})
        api.extend(api, {setting})
    "#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn constructor_call_keeps_invalid_fields_and_values() {
    for (value, expected) in [
        ("{}", DiagnosticCode::MissingFields),
        ("{count = 1, extra = true}", DiagnosticCode::InjectField),
        ("{count = 'wrong'}", DiagnosticCode::AssignTypeMismatch),
    ] {
        let diagnostics = check_source(&format!(
            r#"
            ---@class Options
            ---@field count integer
            ---@class Api
            ---@field put fun(self: Api, options: Options)
            ---@type Api
            local api
            api:put {value}
        "#
        ));
        assert!(
            diagnostics.iter().any(|d| d.code == expected),
            "{value}: {diagnostics:?}"
        );
    }
}

#[test]
fn constructor_call_aligns_colon_defined_methods() {
    let diagnostics = check_source(
        r#"
        ---@class Options
        ---@field count integer
        local api = {}
        ---@param options Options
        function api:put(options) return self, options end
        api:put {count = 1}
        api.put(api, {count = 1})
    "#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn constructor_call_selects_overloads_using_other_arguments() {
    let diagnostics = check_source(
        r#"
        ---@class Left
        ---@field left string
        ---@class Right
        ---@field right integer
        ---@type fun(kind: 'left', options: Left) | fun(kind: 'right', options: Right)
        local put
        put('left', {left = 'ok'})
        put('right', {right = 1})
    "#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn constructor_call_keeps_the_matching_overloads_invalid_constructor() {
    let diagnostics = check_source(
        r#"
        ---@class Left
        ---@field value string
        ---@class Right
        ---@field value integer
        ---@type fun(kind: 'left', options: Left) | fun(kind: 'right', options: Right)
        local put
        put('left', {value = 1})
    "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::AssignTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn constructor_call_substitutes_generic_parameters() {
    let diagnostics = check_source(
        r#"
        ---@type fun<T>(seed: T, options: T[])
        local put
        put('text', {'other'})
        put--[[@<integer>]](1, {2})
    "#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn constructor_call_checks_every_variadic_constructor() {
    let diagnostics = check_source(
        r#"
        ---@class Options
        ---@field count integer
        ---@type fun(...: Options)
        local put
        put({count = 1}, {count = 2})
        put({count = 1}, {count = 'bad'})
    "#,
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::AssignTypeMismatch)
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[test]
fn constructor_call_keeps_explicit_generic_contract() {
    let diagnostics = check_source(
        r#"
        ---@type fun<T>(seed: T, options: T[])
        local put
        put--[[@<integer>]](1, {'wrong'})
    "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::AssignTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn constructor_call_accepts_optional_record_alternatives() {
    let diagnostics = check_source(
        r#"
        ---@class Left
        ---@field left? string
        ---@class Right
        ---@field right? integer
        ---@type fun(options: Left) | fun(options: Right)
        local put
        put({left = 'ok'})
        put({right = 1})
    "#,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn constructor_call_requires_one_complete_overload() {
    let diagnostics = check_source(
        r#"
        ---@class Left
        ---@field left? string
        ---@class Right
        ---@field right? integer
        ---@type fun(options: Left) | fun(options: Right)
        local put
        put({left = 1})
    "#,
    );
    assert!(
        diagnostics.iter().any(|d| matches!(
            d.code,
            DiagnosticCode::AssignTypeMismatch | DiagnosticCode::InjectField
        )),
        "{diagnostics:?}"
    );
}

#[test]
fn constructor_call_does_not_select_an_overload_with_no_parameters() {
    let diagnostics = check_source(
        r#"
        ---@class Options
        ---@field count integer
        ---@type fun() | fun(options: Options)
        local put
        put({count = 'wrong'})
    "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::AssignTypeMismatch),
        "{diagnostics:?}"
    );
}

#[test]
fn constructor_call_discriminator_errors_do_not_depend_on_overload_order() {
    for annotation in [
        "fun(kind:'left', options:Left)|fun(kind:'right', options:Right)",
        "fun(kind:'right', options:Right)|fun(kind:'left', options:Left)",
    ] {
        let diagnostics = check_source(&format!(
            r#"
            ---@class Left
            ---@field value string
            ---@class Right
            ---@field value integer
            ---@type {annotation}
            local put
            put('left', {{value=1}})
        "#
        ));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::AssignTypeMismatch),
            "{annotation}: {diagnostics:?}"
        );
    }
}

#[test]
fn constructor_call_packed_variadics_check_each_structural_slot() {
    for (arguments, valid) in [
        ("{left='ok'}, {right=1}", true),
        ("{left='ok'}, {}", false),
        ("{left='ok'}, {wrong=1}", false),
    ] {
        let diagnostics = check_source(&format!(
            r#"
            ---@class Left
            ---@field left string
            ---@class Right
            ---@field right integer
            ---@param ... [Left,Right]...
            local function put(...) end
            put({arguments})
        "#
        ));
        let structural = diagnostics.iter().any(|d| {
            matches!(
                d.code,
                DiagnosticCode::MissingFields | DiagnosticCode::InjectField
            )
        });
        assert_eq!(!structural, valid, "{arguments}: {diagnostics:?}");
    }
}

#[test]
fn constructor_call_targets_keep_inline_asserted_ownership() {
    for (argument, missing) in [("{}", true), ("{} --[[@as Options]]", false)] {
        let diagnostics = check_source(&format!(
            "---@class Options\n---@field count integer\n---@param options Options\nlocal function put(options) end\nput({argument})"
        ));
        assert_eq!(
            diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::MissingFields),
            missing,
            "{argument}: {diagnostics:?}"
        );
    }
}

#[test]
fn constructor_call_resolves_overloads_differing_only_in_field_value_type() {
    for annotation in [
        "fun(options:Text)|fun(options:Number)",
        "fun(options:Number)|fun(options:Text)",
    ] {
        let diagnostics = check_source(&format!(
            "---@class Text\n---@field value string\n---@class Number\n---@field value integer\n---@type {annotation}\nlocal put\nput({{value=1}})\nput({{value='text'}})"
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

#[test]
fn constructor_call_keeps_asserted_alias_and_union_ownership() {
    for annotation in ["OptionsAlias", "Options|Alternative"] {
        let diagnostics = check_source(&format!(
            "---@class Options\n---@field count integer\n---@class Alternative\n---@field name string\n---@alias OptionsAlias Options\n---@param options Options\nlocal function put(options) end\nput({{}} --[[@as {annotation}]])"
        ));
        assert!(
            !diagnostics.iter().any(|d| matches!(
                d.code,
                DiagnosticCode::MissingFields | DiagnosticCode::InjectField
            )),
            "{annotation}: {diagnostics:?}"
        );
    }
}
