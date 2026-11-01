//! Effect boundaries shared by computed-access guards and definite assignments.
//!
//! Opaque calls preserve facts (as ordinary local narrowing does). Visible writes
//! and known mutators still invalidate them; this is not a purity guarantee.
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaCallExpr, LuaClosureExpr, LuaExpr, LuaFuncStat, LuaIndexExpr,
    LuaNameExpr, LuaSyntaxKind, LuaSyntaxNode, LuaVarExpr,
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
            (name.get_name_text().as_deref() == Some("_ENV") && expected.depends_on_globals())
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

/// Lua evaluates the target table/key before the RHS. A call may overwrite the
/// assigned entry, but it must not change the receiver or key used by later reads.
pub(super) fn assignment_inputs_conflict(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    before: TextSize,
) -> bool {
    let mut owner = expected.clone();
    let last = owner.parts.pop();
    calls_conflict(model, &owner, node, before)
        || matches!(last, Some(Part::Key(key)) if calls_conflict(model, &key, node, before))
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
        if node.text_range().start() >= before || LuaClosureExpr::can_cast(node.kind().into()) {
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

/// Examine completed calls before upper_bound, optionally including direct writes.
/// The node supplies the lower boundary; nested closure bodies are not executed.
fn scan(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    upper_bound: TextSize,
    include_writes: bool,
) -> bool {
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        if node.text_range().start() >= upper_bound || LuaClosureExpr::can_cast(node.kind().into())
        {
            continue;
        }
        if include_writes
            && node.text_range().end() <= upper_bound
            && let Some(assign) = LuaAssignStat::cast(node.clone())
            && assign
                .get_var_and_expr_list()
                .0
                .iter()
                .any(|var| write_conflicts(model, expected, var, upper_bound))
        {
            return true;
        }
        // A function statement writes its closure into the named target, while
        // the closure body remains unevaluated and is skipped by this walk.
        if include_writes
            && node.text_range().end() <= upper_bound
            && let Some(function) = LuaFuncStat::cast(node.clone())
            && let Some(target) = function.get_func_name()
            && write_conflicts(model, expected, &target, upper_bound)
        {
            return true;
        }
        if node.text_range().end() <= upper_bound
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
        // Computed paths retain global names; the shared member-call resolver
        // uses declaration identities. Bridge those representations at this boundary.
        let root = match &expected.root {
            crate::SemanticId::Name(name) => model
                .global_decl(name)
                .unwrap_or_else(|| expected.root.clone()),
            _ => expected.root.clone(),
        };
        let path = (root, keys);
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

#[cfg(test)]
mod function_statement_tests {
    use super::*;
    use crate::VirtualWorkspace;

    #[test]
    fn function_statements_invalidate_matching_computed_loop_and_iteration_paths() {
        for (read, target, conflict) in [
            ("rows[key]", "rows.handler", true),
            ("rows[key]", "other.handler", false),
            ("rows.handler", "rows.handler.child", false),
        ] {
            let mut ws = VirtualWorkspace::new();
            let file = ws.def(&format!("local rows={{}}; local other={{}}; local key='handler'; for i=1,10 do local observed={read}; function {target}() end end"));
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == read)
                .unwrap();
            let expected = access(&model, &LuaExpr::IndexExpr(index.clone())).unwrap();
            let loop_stat = chunk
                .descendants::<emmylua_parser::LuaForStat>()
                .next()
                .unwrap();
            assert_eq!(
                loop_conflicts(&model, &expected, loop_stat.syntax()),
                conflict,
                "{target}"
            );
            let block = loop_stat.get_block().unwrap();
            assert_eq!(
                iteration_conflicts(&model, &expected, block.syntax(), block.get_range().end()),
                conflict,
                "{target}"
            );
        }
    }
    #[test]
    fn computed_rawset_keys_terminate_and_invalidate_possible_matches() {
        for key in ["rows[key]", "rows[key][1]"] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            let source = format!(
                "---@param rows table<string, string[]>\n---@param key string\nlocal function f(rows,key) if rows[key] then rawset(rows, {key}, nil); local observed=rows[key] end end"
            );
            let file = ws.def(&source);
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .filter(|index| index.get_text() == "rows[key]")
                .last()
                .unwrap();
            let call = chunk
                .descendants::<LuaCallExpr>()
                .find(|call| call.get_text().starts_with("rawset("))
                .unwrap();
            let expected = access(&model, &LuaExpr::IndexExpr(index)).unwrap();
            for _ in 0..2 {
                assert!(
                    calls_conflict(&model, &expected, call.syntax(), call.get_range().end()),
                    "{key}"
                );
            }
        }
    }
    #[test]
    fn environment_replacement_only_affects_global_computed_dependencies() {
        for (read, conflict) in [
            ("rows[key]", false),
            ("storage[key]", true),
            ("rows[global_key]", true),
            ("rows[local_key.name]", false),
        ] {
            let mut ws = VirtualWorkspace::new();
            let source = format!(
                "local rows={{}}; local key='x'; local local_key={{name='x'}}; local observed={read}; _ENV={{}}"
            );
            let file = ws.def(&source);
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == read)
                .unwrap();
            let expected = access(&model, &LuaExpr::IndexExpr(index)).unwrap();
            let assignment = chunk.descendants::<LuaAssignStat>().last().unwrap();
            let vars = assignment.get_var_and_expr_list().0;
            assert_eq!(
                write_conflicts(&model, &expected, &vars[0], assignment.get_range().end()),
                conflict,
                "{read}"
            );
        }
    }
}
