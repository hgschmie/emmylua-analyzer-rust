#![cfg(test)]
use crate::handlers::{
    completion::completion, definition::definition, hover::hover,
    test_lib::ProviderVirtualWorkspace,
};
use googletest::prelude::*;
use lsp_types::{
    CompletionResponse, CompletionTriggerKind, GotoDefinitionResponse, HoverContents, Position,
};
use tokio_util::sync::CancellationToken;

const DEFS: &str = "---@meta\n---@class Property\n--- A transformed property.\n---@field (read) value number\n---@field (write) value 'coal'|'wood'\n---@type Property\nproperty = {}\n";

#[gtest]
fn asymmetric_fields_hover_and_navigation() {
    let mut ws = ProviderVirtualWorkspace::new();
    ws.def_file("defs.lua", DEFS);
    let file = ws.def_file("use.lua", "local x = property.value\nproperty.value = 'coal'\nlocal y = property.value\nproperty['value'] = 'wood'");
    for (line, col, target_line) in [(0, 20, 3), (1, 11, 4), (2, 20, 3), (3, 12, 4)] {
        let result = hover(&ws.analysis, file, Position::new(line, col)).expect("hover");
        let HoverContents::Markup(text) = result.contents else {
            panic!("markup");
        };
        assert!(text.value.contains("Read: number"), "{}", text.value);
        assert!(text.value.contains("Write:"), "{}", text.value);
        assert!(text.value.contains("coal"), "{}", text.value);
        assert!(
            text.value.contains("A transformed property."),
            "{}",
            text.value
        );
        let definition =
            definition(&ws.analysis, file, Position::new(line, col)).expect("definition");
        let locations = match definition {
            GotoDefinitionResponse::Scalar(location) => vec![location],
            GotoDefinitionResponse::Array(locations) => locations,
            _ => panic!("locations"),
        };
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].range.start.line, target_line);
    }
}

#[gtest]
fn asymmetric_fields_completion() -> Result<()> {
    let mut ws = ProviderVirtualWorkspace::new();
    ws.def_file("defs.lua", DEFS);
    for (source, expected, count) in [
        ("property.<??>", "value", 1),
        ("property.value = <??>", "coal", 1),
    ] {
        let (source, position) = ProviderVirtualWorkspace::handle_file_content(source)?;
        let file = ws.def(&source);
        let response = completion(
            &ws.analysis,
            file,
            position,
            CompletionTriggerKind::INVOKED,
            CancellationToken::new(),
        )
        .expect("completion");
        let items = match response {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        };
        if expected == "value" {
            let item = items.iter().find(|item| item.label == "value").unwrap();
            let resolved = crate::handlers::completion::completion_resolve(
                &ws.analysis,
                item.clone(),
                crate::context::ClientId::Intellij,
            );
            let detail = serde_json::to_string(&resolved).unwrap();
            assert!(
                detail.contains("Read: number")
                    && detail.contains("Write:")
                    && detail.contains("coal"),
                "{detail}"
            );
        }
        assert_eq!(
            items
                .iter()
                .filter(|item| item.label.contains(expected))
                .count(),
            count,
            "{items:?}"
        );
    }
    Ok(())
}
