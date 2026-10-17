use crate::handlers::hover::hover;
use crate::handlers::test_lib::{
    ProviderVirtualWorkspace, VirtualHoverResult, VirtualLocation, check,
};
use googletest::prelude::*;
use lsp_types::{HoverContents, Position};

#[gtest]
fn array_member_definition_resolves_the_expression() -> Result<()> {
    for entry in [
        "function() return Constants.<??>getStationName() end",
        "Constants.<??>getStationName()",
        "{ function() return Constants.<??>getStationName() end }",
        "[1] = function() return Constants.<??>getStationName() end",
        "named = function() return Constants.<??>getStationName() end",
    ] {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            &format!("local Constants = {{}}\nfunction Constants.getStationName() return '' end\nlocal callbacks = {{ {entry} }}"),
            vec![VirtualLocation { file: "".into(), line: 1 }],
        ));
    }
    Ok(())
}

#[gtest]
fn array_member_references_resolve_the_called_function() -> Result<()> {
    let mut ws = ProviderVirtualWorkspace::new();
    check!(ws.check_references(
        "local Constants = {}\nfunction Constants.getStationName() return '' end\nlocal callbacks = {\n function() return Constants.<??>getStationName() end,\n function() return Constants.getStationName() end,\n Constants.getStationName,\n}\nConstants.getStationName()",
        vec![],
        [1, 3, 4, 5, 7].into_iter().map(|line| VirtualLocation { file: "".into(), line }).collect(),
    ));
    Ok(())
}

#[gtest]
fn array_member_function_hover_matches_an_ordinary_call() {
    let mut ws = ProviderVirtualWorkspace::new();
    let source = "local Constants = {}\n---@return string\nfunction Constants.getStationName() return '' end\nConstants.getStationName()\nlocal callbacks = {\n function() return Constants.getStationName() end,\n { Constants.getStationName() },\n}";
    let file = ws.def(source);
    let at = |line: usize| {
        let column = source
            .lines()
            .nth(line)
            .unwrap()
            .find("getStationName")
            .unwrap();
        hover(
            &ws.analysis,
            file,
            Position::new(line as u32, column as u32 + 1),
        )
        .unwrap()
        .contents
    };
    let expected = at(3);
    let HoverContents::Markup(content) = &expected else {
        panic!("expected markdown");
    };
    assert!(
        content.value.contains("getStationName"),
        "{}",
        content.value
    );
    for line in [5, 6] {
        expect_eq!(at(line), expected);
    }
}

#[gtest]
fn array_member_field_hover_retains_the_read_type() -> Result<()> {
    for entry in [
        "function() return settings.<??>count end",
        "settings.<??>count",
        "{ settings.<??>count }",
    ] {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            &format!("---@class Settings\n---@field count integer\n---@type Settings\nlocal settings\nlocal values = {{ {entry} }}"),
            VirtualHoverResult { value: "```lua\n(field) count: integer\n```".into() },
        ));
    }
    Ok(())
}

#[gtest]
fn array_member_explicit_keys_still_have_definitions() -> Result<()> {
    for key in ["<??>named", "[1<??>0]"] {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            &format!("local callbacks = {{ {{ {key} = function() end }} }}"),
            vec![VirtualLocation {
                file: "".into(),
                line: 0
            }],
        ));
    }
    Ok(())
}
