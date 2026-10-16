use super::super::hover::hover;
use crate::handlers::test_lib::ProviderVirtualWorkspace;
use lsp_types::HoverContents;

fn hover_text(definitions: &str, source: &str, separate: bool) -> String {
    let mut ws = ProviderVirtualWorkspace::new();
    let source = if separate {
        ws.def(definitions);
        source.to_string()
    } else {
        format!("{definitions}\n{source}")
    };
    let (content, position) = ProviderVirtualWorkspace::handle_file_content(&source).unwrap();
    let file = ws.def(&content);
    let result = hover(&ws.analysis, file, position).expect("hover should resolve");
    let HoverContents::Markup(markup) = result.contents else {
        panic!("expected markup hover");
    };
    markup.value
}

fn code(text: &str) -> &str {
    text.strip_prefix("```lua\n")
        .unwrap()
        .split("\n```")
        .next()
        .unwrap()
}

const DEFINITIONS: &str = r#"
---@enum TickState
local states = { reset = 0, running = 1 }
---@class Storage
---@field tick_state TickState Current stage.
---@type Storage
storage = {}
"#;

#[test]
fn declared_field_hover_keeps_enum_contract_on_assignment() {
    for separate in [false, true] {
        for source in [
            "storage.<??>tick_state = 0",
            "storage.tick_state = storage.tick_state + 1\nstorage.<??>tick_state = 0",
        ] {
            let text = hover_text(DEFINITIONS, source, separate);
            assert_eq!(code(&text), "(field) tick_state: TickState = 0", "{text}");
            assert!(text.contains("Current stage."), "{text}");
        }
    }
}

#[test]
fn declared_field_hover_keeps_alias_contract() {
    let text = hover_text(
        "---@alias Count integer\n---@class Counter\n---@field count Count\n---@type Counter\nlocal counter = {}",
        "counter.<??>count = 2",
        false,
    );
    assert_eq!(code(&text), "(field) count: Count = 2");
}

#[test]
fn declared_field_hover_does_not_change_untyped_members() {
    let text = hover_text("local object = {}", "object.<??>count = 2", false);
    assert_eq!(code(&text), "(field) count: number = 2");
}

#[test]
fn declared_field_hover_preserves_explicit_runtime_annotation() {
    let text = hover_text(
        "---@class Object\n---@field value any\n---@type Object\nlocal object = {}",
        "---@type string\nobject.<??>value = 'value'",
        false,
    );
    assert_eq!(code(&text), "(field) value: string = \"value\"");
}

#[test]
fn declared_field_hover_preserves_optional_and_generic_projection() {
    let optional = hover_text(
        "---@class Object\n---@field value? string\n---@type Object\nlocal object = {}",
        "object.<??>value = 'value'",
        false,
    );
    assert_eq!(code(&optional), "(field) value: string = \"value\"");
    let generic = hover_text(
        "---@class Box<T>\n---@field value T\n---@type Box<string>\nlocal box = {}",
        "box.<??>value = 'value'",
        false,
    );
    assert_eq!(code(&generic), "(field) value: string = \"value\"");
}

#[test]
fn declared_field_hover_projects_nested_generic_fields() {
    let text = hover_text(
        &format!(
            "{DEFINITIONS}\n---@class Box<T>\n---@field value T\n---@class Parent\n---@field child Box<TickState>\n---@type Parent\nlocal parent = {{}}"
        ),
        "parent.child.<??>value = 0",
        false,
    );
    assert_eq!(code(&text), "(field) value: TickState = 0");
}

#[test]
fn declared_field_hover_uses_the_scoped_receiver() {
    let text = hover_text(
        &format!("{DEFINITIONS}\n---@class OtherStorage\n---@field tick_state string"),
        "do\n---@type OtherStorage\nlocal storage = {}\nstorage.<??>tick_state = 'ready'\nend",
        false,
    );
    assert_eq!(code(&text), "(field) tick_state: string = \"ready\"");
}
