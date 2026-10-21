use crate::handlers::{hover::hover, test_lib::ProviderVirtualWorkspace};
use lsp_types::HoverContents;

const API: &str = r#"
---@class Entity
local Entity = {
    ---Get all connectors.
    ---@param create boolean
    ---@return table<integer, string>
    get_connectors = function(create) end,
}
---@class Other
local Other = {
    ---Get other connectors.
    ---@param create boolean
    ---@return integer
    get_connectors = function(create) end,
}
"#;

fn hover_text(api: &str, source: &str, separate: bool) -> String {
    let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
    let source = if separate {
        ws.def(api);
        source.to_owned()
    } else {
        format!("{api}\n{source}")
    };
    let (content, position) = ProviderVirtualWorkspace::handle_file_content(&source).unwrap();
    let file = ws.def(&content);
    let result = hover(&ws.analysis, file, position).unwrap();
    let HoverContents::Markup(markup) = result.contents else {
        panic!("expected markup")
    };
    markup.value
}

#[test]
fn assigned_receiver_retains_member_documentation() {
    for separate in [false, true] {
        for assignments in [
            "value = entity",
            "if flag then value = entity else value = entity end",
        ] {
            let source = format!(
                r#"
---@param entity Entity
---@param flag boolean
local function test(entity, flag)
    local value
    {assignments}
    value.<??>get_connectors(true)
end
"#
            );
            let text = hover_text(API, &source, separate);
            assert!(
                text.contains("Entity.get_connectors(create: boolean) -> table<integer,string>"),
                "{text}"
            );
            assert!(text.contains("Get all connectors."), "{text}");
        }
    }
}

#[test]
fn reassigned_receiver_uses_current_class_documentation() {
    let text = hover_text(
        API,
        r#"
---@param entity Entity
---@param other Other
local function test(entity, other)
    local value
    value = entity
    value = other
    value.<??>get_connectors(true)
end
"#,
        true,
    );
    assert!(
        text.contains("Other.get_connectors(create: boolean) -> integer"),
        "{text}"
    );
    assert!(text.contains("Get other connectors."), "{text}");
    assert!(!text.contains("Get all connectors."), "{text}");
}

#[test]
fn flow_receiver_does_not_pick_documentation_from_ambiguous_union() {
    let text = hover_text(
        API,
        r#"
---@param entity Entity
---@param other Other
---@param flag boolean
local function test(entity, other, flag)
    local value
    if flag then value = entity else value = other end
    value.<??>get_connectors(true)
end
"#,
        true,
    );
    assert!(
        !text.contains("Get all connectors.") && !text.contains("Get other connectors."),
        "{text}"
    );
}

#[test]
fn flow_receiver_preserves_generic_member_projection() {
    let text = hover_text(
        r#"
---@class Box<T>
---@field value T Stored value.
"#,
        r#"
---@param box Box<string>
local function test(box)
    local value
    value = box
    print(value.<??>value)
end
"#,
        true,
    );
    assert!(text.contains("value: string"), "{text}");
    assert!(text.contains("Stored value."), "{text}");
}

#[test]
fn nullable_flow_receiver_keeps_nil_diagnostic() {
    use emmylua_code_analysis::{DiagnosticCode, VirtualWorkspace};
    let source = format!(
        r#"
{API}
---@param entity Entity?
local function test(entity)
    local value
    value = entity
    value.get_connectors(true)
end
"#
    );
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source));
    let text = hover_text(
        "",
        &source.replace("value.get_connectors", "value.<??>get_connectors"),
        true,
    );
    assert!(text.contains("Get all connectors."), "{text}");
}
