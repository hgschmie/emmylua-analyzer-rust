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

#[gtest]
fn constructor_hover_module_annotation_preserves_member_contract() -> Result<()> {
    for owner in [
        "",
        "---@type string[]\n",
        "---@type table<string,boolean>\n",
        "---@class Container\n---@field entry boolean\nlocal Container = {}\n---@type Container\n",
    ] {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("known.lua", "return 42");
        let source =
            format!("{owner}local values = {{\n---@module 'known'\n <??>entry = missing\n}}");
        let file = ws.def(&source.replace("<??>", ""));
        let model = ws.analysis.semantic_model(file);
        let facts = model.file_facts().unwrap();
        let member = facts
            .members
            .iter()
            .find(|member| {
                member.key.name() == Some("entry")
                    && !matches!(member.owner, emmylua_code_analysis::SemanticId::TypeDef(_))
            })
            .unwrap();
        assert_eq!(member.module_path.as_deref(), Some("known"));
        let typ = model.type_of_member(&member.id).unwrap();
        assert!(matches!(typ, emmylua_code_analysis::LuaType::ModuleRef(_)));
        check!(ws.check_hover(
            &source,
            VirtualHoverResult {
                value: "```lua\n(field) entry: module\n```".into()
            }
        ));
    }
    Ok(())
}

#[gtest]
fn constructor_hover_user_generic_table_uses_declared_field_contract() -> Result<()> {
    for (name, modifier, with_std) in [
        ("Box", "", false),
        ("table", "", false),
        ("Table", "", true),
        ("Custom.table", "", false),
        ("Custom.table", "", true),
        ("table", "(internal) ", false),
        ("table", "(internal) ", true),
    ] {
        for element in ["Setting", "string"] {
            let mut ws = if with_std {
                ProviderVirtualWorkspace::new_with_init_std_lib()
            } else {
                ProviderVirtualWorkspace::new()
            };
            check!(ws.check_hover(
                &format!(
                    "{TYPES}\n---@class {modifier}{name}<K,V>\n---@field entry K\n---@type {name}<{element},boolean>\nlocal values = {{ <??>entry = missing }}"
                ),
                VirtualHoverResult {value: format!("```lua\n(field) entry: {element}\n```{}", if name == "Custom.table" { "\n\n&nbsp;&nbsp;in class `Custom.table`" } else { "" })}
            ));
        }
    }
    Ok(())
}

#[gtest]
fn constructor_hover_module_annotation_supplies_nested_contract() -> Result<()> {
    for (value, expected) in [
        ("{ <??>child = missing }", "(field) child: integer"),
        ("<??>{ child = missing }", "ModuleConfig"),
        ("{ child = missing <??>}", "ModuleConfig"),
    ] {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "known.lua",
            "---@class ModuleConfig\n---@field child integer\nlocal config = {}\nreturn config",
        );
        check!(ws.check_hover(
            &format!("---@class Other\n---@field child boolean\nlocal Other = {{}}\n---@type table<string,Other>\nlocal values = {{\n---@module 'known'\n entry = {value}\n}}"),
            VirtualHoverResult {value: format!("```lua\n{expected}\n```")}
        ));
    }
    Ok(())
}

#[gtest]
fn constructor_hover_unknown_array_key_does_not_borrow_element_field() -> Result<()> {
    check_case(
        "---@type Setting[]\nlocal settings = { <??>key = missing }",
        "(field) key: unknown",
    )
}

#[gtest]
fn constructor_hover_builtin_table_contract_is_preserved() -> Result<()> {
    for with_std in [false, true] {
        for (key, expected) in [("string", "Setting"), ("integer", "unknown")] {
            let mut ws = if with_std {
                ProviderVirtualWorkspace::new_with_init_std_lib()
            } else {
                ProviderVirtualWorkspace::new()
            };
            check!(ws.check_hover(
                &format!("{TYPES}\n---@type table<{key},Setting>\nlocal values = {{ <??>entry = missing }}"),
                VirtualHoverResult {value: format!("```lua\n(field) entry: {expected}\n```")}
            ));
        }
    }
    Ok(())
}

#[gtest]
fn constructor_definition_keeps_unannotated_keys_and_required_values() -> Result<()> {
    let mut ws = ProviderVirtualWorkspace::new();
    check!(ws.check_definition(
        "local values = { ['<??>entry'] = 1 }",
        vec![VirtualLocation {
            file: "".into(),
            line: 0
        }]
    ));
    let module_file = "tests/fixtures/constructor_context/known.lua";
    ws.def_file(
        module_file,
        include_str!("../../../tests/fixtures/constructor_context/known.lua"),
    );
    check!(ws.check_definition(
        "local values = { entry = require('<??>tests.fixtures.constructor_context.known') }",
        vec![VirtualLocation {
            file: "known.lua".into(),
            line: 0
        }]
    ));
    Ok(())
}

#[gtest]
fn constructor_hover_unknown_computed_key_keeps_nested_value_inferred() -> Result<()> {
    use emmylua_parser::{LuaAstNode, LuaIndexKey, LuaTableField};

    for owner in [
        "Setting[]",
        "table<string,Setting>",
        "{ [string]: Setting }",
    ] {
        let mut ws = ProviderVirtualWorkspace::new();
        let source = format!(
            "{TYPES}\n---@type {owner}\nlocal settings = {{ [missing_key] = {{ <??>key = true }} }}"
        );
        let file = ws.def(&source.replace("<??>", ""));
        let model = ws.analysis.semantic_model(file);
        let key = model
            .chunk()
            .unwrap()
            .syntax()
            .descendants()
            .filter_map(LuaTableField::cast)
            .find_map(|field| match field.get_field_key()? {
                LuaIndexKey::Expr(expr) => Some(expr),
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            model.type_of_expr(key.get_syntax_id()),
            emmylua_code_analysis::LuaType::Unknown
        ));
        check!(ws.check_hover(
            &source,
            VirtualHoverResult {
                value: "```lua\n(field) key: boolean = true\n```".into()
            }
        ));
    }
    Ok(())
}

#[gtest]
fn constructor_definition_annotated_quoted_key_ignores_enclosing_call() -> Result<()> {
    for literal in ["{ ['<??>entry'] = 1 }", "consume({ ['<??>entry'] = 1 })"] {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_definition(
            &format!("---@class Record\n---@field entry integer\n---@param value Record\nlocal function consume(value) end\n---@type Record\nlocal values = {literal}"),
            vec![VirtualLocation {file: "".into(), line: 1}, VirtualLocation {file: "".into(), line: 5}]
        ));
    }
    Ok(())
}

#[gtest]
fn constructor_hover_computed_key_domain_controls() -> Result<()> {
    for (owner, key_type, projected) in [
        ("Setting[]", "integer", true),
        ("Setting[]", "integer?", true),
        ("Setting[]", "string", false),
        ("Setting[]", "nil", false),
        ("Setting[]", "any", true),
        ("table<string,Setting>", "string", true),
        ("table<string,Setting>", "string?", true),
        ("table<string,Setting>", "integer", false),
        ("table<string,Setting>", "nil", false),
        ("table<string,Setting>", "any", true),
        ("{ [string]: Setting }", "string", true),
        ("{ [string]: Setting }", "integer", false),
    ] {
        check_case(
            &format!(
                "---@type {key_type}\nlocal index\n---@type {owner}\nlocal settings = {{ [index] = {{ <??>key = true }} }}"
            ),
            if projected {
                "(field) key: string = true"
            } else {
                "(field) key: boolean = true"
            },
        )?;
    }
    check_case(
        "---@type table<string,Setting>\nlocal settings = { [42] = { <??>key = missing } }",
        "(field) key: unknown",
    )?;
    Ok(())
}

#[gtest]
fn constructor_hover_synthetic_union_keeps_all_value_contracts() -> Result<()> {
    check_case(
        "---@type table<string,string>|table<string,integer>\nlocal settings = { <??>entry = missing }",
        "(field) entry: (string|integer)",
    )
}
