use crate::handlers::{hover::hover, test_lib::ProviderVirtualWorkspace};
use lsp_types::HoverContents;

fn hover_text(source: &str) -> String {
    let mut ws = ProviderVirtualWorkspace::new();
    let (content, position) = ProviderVirtualWorkspace::handle_file_content(source).unwrap();
    let file = ws.def(&content);
    let result = hover(&ws.analysis, file, position).unwrap();
    let HoverContents::Markup(markup) = result.contents else {
        panic!("expected markup")
    };
    markup.value
}

#[test]
fn dot_function_keeps_same_class_argument() {
    for name in ["target", "other", "receiver"] {
        for typ in ["Connector", "self"] {
            let text = hover_text(&format!(
                r#"
---@class Connector
local Connector = {{
    ---Connect another connector.
    ---@param {name} {typ}
    ---@param reach_check boolean
    connect_to = function({name}, reach_check) end,
}}
---@type Connector
local connector
connector.<??>connect_to(connector, false)
"#
            ));
            assert!(
                text.contains(&format!("Connector.connect_to({name}:")),
                "{text}"
            );
            assert!(!text.contains("Connector:connect_to"), "{text}");
            assert!(text.contains("Connect another connector."), "{text}");
        }
    }
}

#[test]
fn explicit_self_names_display_as_methods() {
    for name in ["self", "_self"] {
        for typ in ["Connector", "string", "any"] {
            let text = hover_text(&format!(
                r#"
---@class Connector
---@field connect_to fun({name}: {typ}, other: Connector)
local Connector = {{}}
---@type Connector
local connector
local fn = connector.<??>connect_to
"#
            ));
            assert!(
                text.contains("Connector:connect_to(other: Connector)"),
                "{text}"
            );
            assert!(!text.contains(&format!("{name}:")), "{text}");
        }
    }
}

#[test]
fn runtime_explicit_self_names_display_as_methods() {
    for name in ["self", "_self"] {
        let text = hover_text(&format!(
            r#"
---@class Connector
local Connector = {{}}
---@param {name} Connector
---@param other Connector
function Connector.connect_to({name}, other) end
---@type Connector
local connector
local fn = connector.<??>connect_to
"#
        ));
        assert!(
            text.contains("Connector:connect_to(other: Connector)"),
            "{text}"
        );
    }
}

#[test]
fn colon_definition_keeps_its_first_explicit_argument() {
    let text = hover_text(
        r#"
---@class Connector
local Connector = {}
---@param other Connector
function Connector:connect_to(other) end
---@type Connector
local connector
local fn = connector.<??>connect_to
"#,
    );
    assert!(
        text.contains("Connector:connect_to(other: Connector)"),
        "{text}"
    );
}

#[test]
fn later_self_parameter_does_not_make_a_method() {
    let text = hover_text(
        r#"
---@class Connector
---@field connect_to fun(other: Connector, self: Connector)
local Connector = {}
---@type Connector
local connector
local fn = connector.<??>connect_to
"#,
    );
    assert!(
        text.contains("Connector.connect_to(other: Connector, self: Connector)"),
        "{text}"
    );
}

#[test]
fn overloads_classify_each_signature_using_explicit_self_only() {
    let text = hover_text(
        r#"
---@class Connector
---@field connect_to fun(target: Connector, reach_check: boolean)
---@field connect_to fun(_self: Connector, count: integer)
local Connector = {}
---@type Connector
local connector
local fn = connector.<??>connect_to
"#,
    );
    assert!(
        text.contains("Connector.connect_to(target: Connector, reach_check: boolean)"),
        "{text}"
    );
    assert!(
        text.contains("Connector:connect_to(count: integer)"),
        "{text}"
    );
}
