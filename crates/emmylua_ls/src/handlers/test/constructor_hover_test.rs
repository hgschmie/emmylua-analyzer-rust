use crate::handlers::test_lib::{
    ProviderVirtualWorkspace, VirtualHoverResult, VirtualLocation, check,
};
use googletest::prelude::*;

const TYPES: &str = r#"
---@class Setting
---@field key string
---@field value boolean
---@alias Group table<string, Setting>
---@alias SettingsMap table<string, Group>
"#;

fn check_case(source: &str, expected: &str) -> Result<()> {
    let mut ws = ProviderVirtualWorkspace::new();
    check!(ws.check_hover(
        &format!("{TYPES}\n{source}"),
        VirtualHoverResult {
            value: format!("```lua\n{expected}\n```")
        }
    ));
    Ok(())
}

#[gtest]
fn constructor_hover_dictionary_value_contract() -> Result<()> {
    check_case(
        "---@type SettingsMap\nlocal settings = { <??>startup = {} }",
        "(field) startup: Group",
    )
}

#[gtest]
fn constructor_hover_nested_dictionary_value_contract() -> Result<()> {
    check_case(
        "---@type SettingsMap\nlocal settings = { startup = { <??>entry = {} } }",
        "(field) entry: Setting",
    )
}

#[gtest]
fn constructor_hover_nested_field_contract() -> Result<()> {
    check_case(
        "---@type SettingsMap\nlocal settings = { startup = { entry = { <??>key = missing } } }",
        "(field) key: string",
    )
}

#[gtest]
fn constructor_hover_computed_key_local() -> Result<()> {
    check_case(
        "---@type string\nlocal name\n---@type Group\nlocal settings = { [<??>name] = {} }",
        "local name: string",
    )
}

#[gtest]
fn constructor_hover_computed_key_member() -> Result<()> {
    check_case(
        "---@class Names\n---@field entry string\n---@type Names\nlocal names\n---@type Group\nlocal settings = { [names.<??>entry] = {} }",
        "(field) entry: string",
    )
}

#[gtest]
fn constructor_hover_computed_key_table_value() -> Result<()> {
    check_case(
        "---@type string\nlocal name\n---@type Group\nlocal settings = { [name] = <??>{} }",
        "Setting",
    )
}

#[gtest]
fn constructor_hover_array_element_contract() -> Result<()> {
    check_case(
        "---@type Setting[]\nlocal settings = { { <??>key = missing } }",
        "(field) key: string",
    )
}

#[gtest]
fn constructor_hover_explicit_field_type_wins() -> Result<()> {
    check_case(
        "---@type Group\nlocal settings = {\n---@type integer\n <??>entry = 42\n}",
        "(field) entry: integer = 42",
    )
}

#[gtest]
fn constructor_hover_invalid_dictionary_key_stays_inferred() -> Result<()> {
    check_case(
        "---@type table<integer, Setting>\nlocal settings = { <??>entry = true }",
        "(field) entry: boolean = true",
    )
}

#[gtest]
fn constructor_hover_computed_key_definition() -> Result<()> {
    let mut ws = ProviderVirtualWorkspace::new();
    check!(ws.check_definition(
        "local name = 'one'\nlocal values = { [<??>name] = {} }",
        vec![VirtualLocation {
            file: "".into(),
            line: 0
        }]
    ));
    Ok(())
}

#[gtest]
fn constructor_hover_call_argument_contract() -> Result<()> {
    check_case(
        "---@param settings SettingsMap\nlocal function configure(settings) end\nconfigure({startup={<??>entry={}}})",
        "(field) entry: Setting",
    )
}

#[gtest]
fn constructor_hover_multiple_assignment_contract() -> Result<()> {
    check_case(
        "---@type string, SettingsMap\nlocal first, settings = '', {startup={<??>entry={}}}",
        "(field) entry: Setting",
    )
}

#[gtest]
fn constructor_hover_quoted_and_numeric_keys() -> Result<()> {
    for key in ["['entry']", "[42]"] {
        check_case(
            &format!(
                "---@type table<string|integer, Setting>\nlocal settings = {{ {key} = {{ <??>key = missing }} }}"
            ),
            "(field) key: string",
        )?;
    }
    Ok(())
}

#[gtest]
fn constructor_hover_enum_dictionary_contract() -> Result<()> {
    check_case(
        "---@enum Kind\nlocal Kind = { startup = 'startup' }\n---@type table<Kind, Group>\nlocal settings = { <??>startup = {} }",
        "(field) startup: Group",
    )
}

#[gtest]
fn constructor_hover_generic_alias_contract() -> Result<()> {
    check_case(
        "---@alias Map<T> table<string,T>\n---@type Map<Setting>\nlocal settings = { <??>entry = {} }",
        "(field) entry: Setting",
    )
}

#[gtest]
fn constructor_hover_alias_cycle_terminates() -> Result<()> {
    check_case(
        "---@alias Cycle Cycle\n---@type Cycle\nlocal settings = { <??>entry = true }",
        "(field) entry: boolean = true",
    )
}

#[gtest]
fn constructor_hover_computed_key_references() -> Result<()> {
    let mut ws = ProviderVirtualWorkspace::new();
    check!(
        ws.check_references(
            "local name = 'one'\nlocal values = { [<??>name] = {} }\nprint(name)",
            vec![],
            [0, 1, 2]
                .into_iter()
                .map(|line| VirtualLocation {
                    file: "".into(),
                    line
                })
                .collect()
        )
    );
    Ok(())
}

#[gtest]
fn constructor_hover_named_array_key_stays_inferred() -> Result<()> {
    check_case(
        "---@alias Items Setting[]\n---@type Items\nlocal settings = { <??>key = true }",
        "(field) key: boolean = true",
    )
}

#[gtest]
fn constructor_definition_follows_nested_contracts() -> Result<()> {
    for source in [
        "---@type table<string, Entry>\nlocal entries = { first = { <??>value = 1 } }",
        "---@type Entry[]\nlocal entries = { { <??>value = 1 } }",
        "---@type table<string, Entry>\nlocal entries\nentries = { first = { <??>value = 1 } }",
        "---@class Owner\n---@field entries table<string, Entry>\nlocal owner = {}\nowner.entries = { ['first'] = { <??>value = 1 } }",
        "---@param entries table<string, Entry>\nlocal function accept(entries) end\naccept({ first = { <??>value = 1 } })",
        "---@alias Map<T> table<string,T>\n---@type Map<Entry>\nlocal entries = { first = { ['<??>value'] = 1 } }",
        "local key = 'first'\n---@type table<string, Entry>\nlocal entries = { [key] = { <??>value = 1 } }",
        "---@type string, Entry[]\nlocal other, entries = '', {{ <??>value = 1 }}",
    ] {
        let mut ws = ProviderVirtualWorkspace::new();
        let source = format!("---@class Entry\n---@field value integer\n\n{source}");
        let field_line = source
            .lines()
            .position(|line| line.contains("<??>"))
            .unwrap();
        check!(ws.check_definition(
            &source,
            vec![
                VirtualLocation {
                    file: "".into(),
                    line: 1
                },
                VirtualLocation {
                    file: "".into(),
                    line: field_line as u32
                },
            ]
        ));
    }
    Ok(())
}

#[gtest]
fn constructor_definition_preserves_value_and_anonymous_key_targets() -> Result<()> {
    for (source, target) in [
        (
            "local original = 1\n---@type table<string, integer>\nlocal values = { key = <??>original }",
            0,
        ),
        (
            "---@type table<string, integer>\nlocal values = { <??>key = 1 }",
            1,
        ),
        (
            "local original = 'key'\n---@type table<string, integer>\nlocal values = { [<??>original] = 1 }",
            0,
        ),
        (
            "---@class Entry\n---@field value integer\n---@type table<string, Entry>\nlocal values = { entry = { callback = function() local <??>value = 1 end } }",
            3,
        ),
    ] {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            source,
            vec![VirtualLocation {
                file: "".into(),
                line: target
            }]
        ));
    }
    Ok(())
}
