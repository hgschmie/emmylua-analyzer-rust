//! Effect boundaries shared by computed-access guards and definite assignments.
//!
//! Opaque calls preserve facts (as ordinary local narrowing does). Visible writes
//! and known mutators still invalidate them; this is not a purity guarantee.
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaCallExpr, LuaExpr, LuaIndexExpr, LuaNameExpr, LuaSyntaxKind,
    LuaSyntaxNode, LuaVarExpr,
};
use rowan::TextSize;

use super::{
    SemanticModel,
    computed_guard::{Access, Part, access, reference},
};

pub(super) fn write_conflicts(
    model: &SemanticModel,
    expected: &Access,
    var: &LuaVarExpr,
    before: TextSize,
) -> bool {
    match var {
        LuaVarExpr::NameExpr(name) => {
            name.get_name_text().as_deref() == Some("_ENV")
                || reference(model, name).is_some_and(|decl| expected.depends_on(&decl))
        }
        LuaVarExpr::IndexExpr(index) => {
            let Some(target) = access(model, &LuaExpr::IndexExpr(index.clone())) else {
                return true;
            };
            // Changing a key's fields changes the identity of the guarded access.
            if key_write_conflicts(expected, &target) {
                return true;
            }
            // Updating a child does not remove the guarded parent container.
            if expected.root == target.root
                && target.parts.len() > expected.parts.len()
                && target.parts.starts_with(&expected.parts)
            {
                return false;
            }
            // A direct write into a fresh, unescaped local table cannot alias it.
            !(target.parts.len() == 1
                && !expected.depends_on(&target.root)
                && fresh_unescaped_table(model, &target, before))
        }
    }
}

fn key_write_conflicts(expected: &Access, target: &Access) -> bool {
    expected.parts.iter().any(|part| match part {
        Part::Key(key) => key.root == target.root || key_write_conflicts(key, target),
        _ => false,
    })
}

fn fresh_unescaped_table(model: &SemanticModel, target: &Access, before: TextSize) -> bool {
    let Some(facts) = model.file_facts() else {
        return false;
    };
    let Some(decl) = facts.decl_by_id(&target.root) else {
        return false;
    };
    if !decl.kind.is_local()
        || model
            .flow_tree()
            .is_none_or(|tree| tree.has_decl_assignment(&target.root))
    {
        return false;
    }
    let Some(chunk) = model.chunk() else {
        return false;
    };
    let Some(value) = decl
        .value_expr_syntax
        .and_then(|ptr| ptr.to_node_from_root(chunk.syntax()))
    else {
        return false;
    };
    if !matches!(LuaExpr::cast(value), Some(LuaExpr::TableExpr(_))) {
        return false;
    }
    // Reading/writing entries is safe. Passing, returning or aliasing the table
    // itself before this point means it can no longer be treated as independent.
    chunk
        .descendants::<LuaNameExpr>()
        .filter(|name| {
            name.get_position() < before && reference(model, name).as_ref() == Some(&target.root)
        })
        .all(|name| {
            name.syntax()
                .parent()
                .and_then(LuaIndexExpr::cast)
                .and_then(|index| index.get_prefix_expr())
                .is_some_and(|prefix| prefix.syntax() == name.syntax())
        })
}

pub(super) fn calls_conflict(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    before: TextSize,
) -> bool {
    scan(model, expected, node, before, false)
}

pub(super) fn loop_conflicts(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
) -> bool {
    // Include later body writes: they may have run on a preceding iteration.
    scan(model, expected, node, node.text_range().end(), true)
}

/// An iterator establishes existence anew on each iteration. Only preceding
/// effects in its body matter, but a nested loop may already have run its tail.
pub(super) fn iteration_conflicts(
    model: &SemanticModel,
    expected: &Access,
    body: &LuaSyntaxNode,
    before: TextSize,
) -> bool {
    let mut pending = vec![body.clone()];
    while let Some(node) = pending.pop() {
        if node.text_range().start() >= before || node.kind() == LuaSyntaxKind::ClosureExpr.into() {
            continue;
        }
        if matches!(
            node.kind().into(),
            LuaSyntaxKind::ForRangeStat
                | LuaSyntaxKind::ForStat
                | LuaSyntaxKind::WhileStat
                | LuaSyntaxKind::RepeatStat
        ) {
            if loop_conflicts(model, expected, &node) {
                return true;
            }
        } else {
            pending.extend(node.children());
        }
    }
    scan(model, expected, body, before, true)
}

fn scan(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    before: TextSize,
    writes: bool,
) -> bool {
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        if node.text_range().start() >= before || node.kind() == LuaSyntaxKind::ClosureExpr.into() {
            continue;
        }
        if writes
            && node.text_range().end() <= before
            && let Some(assign) = LuaAssignStat::cast(node.clone())
            && assign
                .get_var_and_expr_list()
                .0
                .iter()
                .any(|var| write_conflicts(model, expected, var, before))
        {
            return true;
        }
        if node.text_range().end() <= before
            && let Some(call) = LuaCallExpr::cast(node.clone())
            && call_conflicts(model, expected, &call)
        {
            return true;
        }
        pending.extend(node.children());
    }
    false
}

fn call_conflicts(model: &SemanticModel, expected: &Access, call: &LuaCallExpr) -> bool {
    let mut pending = vec![expected];
    while let Some(expected) = pending.pop() {
        let keys = expected
            .parts
            .iter()
            .map(|part| match part {
                Part::Member(key) => key.clone(),
                Part::Key(key) => {
                    pending.push(key);
                    crate::LuaMemberKey::None
                }
            })
            .collect();
        let path = (expected.root.clone(), keys);
        if super::member_call_effects::calls_conflict_with_path(
            model,
            &path,
            call.syntax(),
            call.get_range().end(),
        ) {
            return true;
        }
    }
    false
}
