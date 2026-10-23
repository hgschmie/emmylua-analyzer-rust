use crate::{DiagnosticCode, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaIfStat, LuaIndexExpr};

fn check_branches(definitions: &str, receiver: &str, truthy: &str, falsy: &str) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(definitions);
    ws.def(&format!(
        "---@param player {receiver}\nfunction run(player)\nif player.flag then yes = player else no = player end\nend"
    ));
    assert_eq!(ws.expr_ty("yes"), ws.ty(truthy), "true: {definitions}");
    assert_eq!(ws.expr_ty("no"), ws.ty(falsy), "false: {definitions}");
}

#[test]
fn member_truthiness_possible_branches() {
    for field in [
        "boolean",
        "boolean?",
        "string?",
        "Object?",
        "any",
        "unknown",
        "Unresolved",
    ] {
        check_branches(
            &format!("---@class Object\n---@class Player\n---@field flag {field}"),
            "Player",
            "Player",
            "Player",
        );
    }
}

#[test]
fn member_truthiness_optional_member_markers() {
    for field in ["Object", "fun(): integer", "integer[]", "{name: string}"] {
        check_branches(
            &format!("---@class Object\n---@class Player\n---@field flag? {field}"),
            "Player",
            "Player",
            "Player",
        );
    }
}

#[test]
fn member_truthiness_definite_branches() {
    for (field, truthy, falsy) in [
        ("string", "Player", "never"),
        ("integer", "Player", "never"),
        ("true", "Player", "never"),
        ("false", "never", "Player"),
        ("nil", "never", "Player"),
    ] {
        check_branches(
            &format!("---@class Player\n---@field flag {field}"),
            "Player",
            truthy,
            falsy,
        );
    }
}

#[test]
fn member_truthiness_alias_fields() {
    for (alias, truthy, falsy) in [
        ("boolean", "Player", "Player"),
        ("string?", "Player", "Player"),
        ("false", "never", "Player"),
        ("string", "Player", "never"),
    ] {
        check_branches(
            &format!("---@alias Flag {alias}\n---@class Player\n---@field flag Flag"),
            "Player",
            truthy,
            falsy,
        );
    }
    check_branches(
        "---@alias Flag<T> T?\n---@class Player\n---@field flag Flag<string>",
        "Player",
        "Player",
        "Player",
    );
    check_branches(
        "---@alias Flag {next: Flag}?\n---@class Player\n---@field flag Flag",
        "Player",
        "Player",
        "Player",
    );
    check_branches(
        "---@class Player\n---@field flag boolean\n---@alias Receiver Player",
        "Receiver",
        "Receiver",
        "Receiver",
    );
}

#[test]
fn member_truthiness_unknown_receivers() {
    for receiver in ["unknown", "any", "Unresolved"] {
        check_branches("", receiver, receiver, receiver);
    }
}

#[test]
fn member_truthiness_inherited_and_read_contracts() {
    check_branches(
        "---@class Base\n---@field flag boolean\n---@class Player: Base",
        "Player",
        "Player",
        "Player",
    );
    check_branches(
        "---@class Player\n---@field (read) flag boolean?\n---@field (write) flag string",
        "Player",
        "Player",
        "Player",
    );
    check_branches(
        "---@class Player\n---@field (read) flag string\n---@field (write) flag boolean?",
        "Player",
        "Player",
        "never",
    );
}

#[test]
fn member_truthiness_discriminates_union_members() {
    check_branches(
        "---@class Yes\n---@field flag true\n---@class No\n---@field flag false\n---@class Both\n---@field flag boolean\n---@class Missing",
        "Yes|No|Both|Missing",
        "Yes|Both",
        "No|Both|Missing",
    );
}

#[test]
fn member_truthiness_short_circuit_keeps_falsy_path() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let source = r#"
        ---@class Item
        ---@field valid_for_read boolean
        ---@class Ghost
        ---@class Entity
        ---@class Player
        ---@field cursor_stack? Item
        ---@field cursor_ghost? Ghost
        ---@field selected? Entity
        ---@param player Player?
        local function run(player)
            if not (player and player.cursor_stack) then return end
            if player.cursor_stack.valid_for_read or player.cursor_ghost or player.selected then return end
        end
    "#;
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::UnnecessaryIf);
    let file = ws.def(source);
    let diagnostics = ws
        .analysis
        .diagnose_file(file, tokio_util::sync::CancellationToken::new())
        .unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let player = ws.ty("Player");
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let condition = chunk
        .descendants::<LuaIfStat>()
        .last()
        .unwrap()
        .get_condition_expr()
        .unwrap();
    let condition_type = model.type_of_expr_at(condition.get_syntax_id(), condition.get_position());
    assert!(condition_type.is_nullable(), "{condition_type:?}");
    let selected = chunk
        .descendants::<LuaIndexExpr>()
        .find(|index| index.get_text() == "player.selected")
        .unwrap();
    let receiver = selected.get_prefix_expr().unwrap();
    assert_eq!(
        model.type_of_expr_at(receiver.get_syntax_id(), receiver.get_position()),
        player
    );
}

#[test]
fn member_truthiness_alias_condition_diagnostics() {
    for (field, unnecessary) in [("boolean?", false), ("string", true), ("false", true)] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            "---@alias Flag {field}\n---@class Player\n---@field flag Flag\n---@param player Player\nlocal function run(player)\nif player.flag then return end\nend"
        );
        assert_eq!(
            !ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, &source),
            unnecessary,
            "{field}"
        );
    }
}

#[test]
fn member_truthiness_prior_guard_remains_definite() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(!ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, "---@class Object\n---@class Player\n---@field flag? Object\n---@param player Player\nlocal function run(player)\nif not player.flag then return end\nif player.flag then return end\nend"));
}

#[test]
fn member_truthiness_logical_optional_union_orders() {
    for receiver in ["Required|Optional", "Optional|Required"] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            "---@class Object\n---@class Required\n---@field flag Object\n---@class Optional\n---@field flag? Object\n---@param player {receiver}\nlocal function run(player)\nif false or player.flag then return end\nend"
        );
        assert!(
            ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, &source),
            "{receiver}"
        );
    }
}

#[test]
fn member_truthiness_keeps_definite_condition_diagnostics() {
    for field in ["string", "false", "nil"] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(!ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, &format!("---@class Player\n---@field flag {field}\n---@param player Player\nlocal function run(player)\nif player.flag then return end\nend")), "{field}");
    }
}
