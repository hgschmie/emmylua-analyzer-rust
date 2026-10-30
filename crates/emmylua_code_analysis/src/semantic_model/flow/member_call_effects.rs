//! Visible call effects for member guards. Opaque calls preserve existing guards.
//! This traversal detects visible writes; it never propagates callee values.
//! Passing a receiver to an opaque call alone does not invalidate guards. Writes
//! behind unresolved alias/receiver resolution may therefore go unreported.

use std::collections::HashSet;

use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaCallExpr, LuaClosureExpr, LuaExpr, LuaIndexExpr, LuaSyntaxNode,
    LuaVarExpr,
};
use rowan::TextSize;

use super::{SemanticModel, guards, index_access_path, path_write_affects, stable_access_path};
use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaClosureId};
use crate::{LuaMemberKey, LuaType};

type Access = (SemanticId, Vec<LuaMemberKey>);
type Bindings = Vec<(SemanticId, Vec<Access>)>;

pub(super) fn calls_conflict(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    node: &LuaSyntaxNode,
    before: TextSize,
) -> bool {
    calls_conflict_after(model, index, node, TextSize::from(0), before)
}

pub(super) fn calls_conflict_after(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    node: &LuaSyntaxNode,
    after: TextSize,
    before: TextSize,
) -> bool {
    let expr = LuaExpr::IndexExpr(index.clone());
    let Some(expected) =
        stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
    else {
        return false;
    };
    scan(model, &expected, node, after, before)
}

/// Inspect all direct writes in a visible body before expanding its callees.
/// Repeated callee/binding states terminate recursion. Only receiver prefixes
/// capable of affecting the queried member are retained, so recursive descendant
/// arguments cannot create an unbounded sequence of distinct states.
fn scan(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    after: TextSize,
    before: TextSize,
) -> bool {
    let mut bodies = vec![(node.clone(), after, before, Bindings::new(), false)];
    let mut seen = HashSet::new();
    while let Some((body, after, before, bindings, writes)) = bodies.pop() {
        let mut pending = vec![body];
        let mut calls = Vec::new();
        while let Some(node) = pending.pop() {
            if node.text_range().start() >= before
                || node.text_range().end() <= after
                || LuaClosureExpr::can_cast(node.kind().into())
            {
                continue;
            }
            if writes
                && node.text_range().end() <= before
                && let Some(assign) = LuaAssignStat::cast(node.clone())
                && assign
                    .get_var_and_expr_list()
                    .0
                    .iter()
                    .any(|var| write_conflicts(model, expected, var, &bindings))
            {
                return true;
            }
            if node.text_range().end() <= before
                && let Some(call) = LuaCallExpr::cast(node.clone())
            {
                if mutator_conflicts(model, expected, &call, &bindings) {
                    return true;
                }
                calls.push(call);
            }
            pending.extend(node.children());
        }
        for call in calls {
            for closure in local_callees(model, &call) {
                let Some(body) = closure.get_block() else {
                    continue;
                };
                let mut nested = bindings.clone();
                let args = call
                    .get_args_list()
                    .map(|args| args.get_args().collect::<Vec<_>>())
                    .unwrap_or_default();
                if let Some(params) = closure.get_params_list() {
                    for (index, param) in params.get_params().enumerate() {
                        let id = SemanticId::decl(model.file_id(), param.get_range());
                        let mut paths = args.get(index).map_or_else(Vec::new, |arg| {
                            access_paths(model, arg, &bindings, expected)
                        });
                        // Following an access can only append keys. An unrelated
                        // root, divergent prefix or descendant cannot write this
                        // member; explicit rebindings and captured roots are
                        // resolved independently when the body is inspected.
                        paths.retain(|path| path_write_affects(path, &expected.0, &expected.1));
                        paths.sort_by(|left, right| left.1.cmp(&right.1));
                        paths.dedup();
                        nested.retain(|(bound, _)| *bound != id);
                        // Empty bindings still identify formals. Do not revive an
                        // earlier recursive actual when this argument is absent.
                        nested.push((id, paths));
                    }
                }
                // These are same-file formal declarations, identified by position.
                nested.sort_by_key(|(id, _)| match id {
                    SemanticId::Decl(key) => Some(key.name_range.start()),
                    _ => None,
                });
                if seen.insert((closure.get_syntax_id(), nested.clone())) {
                    bodies.push((
                        body.syntax().clone(),
                        TextSize::from(0),
                        body.get_range().end(),
                        nested,
                        true,
                    ));
                }
            }
        }
    }
    false
}

fn access_paths(
    model: &SemanticModel,
    expr: &LuaExpr,
    bindings: &Bindings,
    expected: &Access,
) -> Vec<Access> {
    let mut pending = vec![(expr.clone(), Vec::new())];
    let mut seen = HashSet::new();
    let mut paths = HashSet::new();
    while let Some((expr, suffix)) = pending.pop() {
        if !seen.insert((expr.get_syntax_id(), suffix.clone())) {
            continue;
        }
        let Some((root, mut keys)) =
            stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
        else {
            continue;
        };
        keys.extend(suffix);
        // Alias expansion can only prepend keys. A longer path cannot affect
        // this member, and pruning it makes recursive descendant aliases finite.
        if keys.len() > expected.1.len() {
            continue;
        }
        let initial = bindings.iter().find(|(param, _)| *param == root);
        let alias = if initial.is_none() {
            formal_alias_value(model, &root, bindings)
        } else {
            None
        };
        if initial.is_none() && alias.is_none() {
            let path = (root, keys);
            if path_write_affects(&path, &expected.0, &expected.1) {
                paths.insert(path);
            }
            continue;
        }
        for value in reaching_values(model, &root, &expr) {
            if let Some(value) = value.or_else(|| alias.clone()) {
                pending.push((value, keys.clone()));
            } else if let Some((_, initial)) = initial {
                for (root, prefix) in initial {
                    let mut path = (root.clone(), prefix.clone());
                    path.1.extend(keys.clone());
                    if path_write_affects(&path, &expected.0, &expected.1) {
                        paths.insert(path);
                    }
                }
            }
        }
    }
    paths.into_iter().collect()
}

/// Find a name-only initializer chain to a bound formal without reviving detached member paths.
fn formal_alias_value(
    model: &SemanticModel,
    root: &SemanticId,
    bindings: &Bindings,
) -> Option<LuaExpr> {
    let facts = model.file_facts()?;
    let chunk = model.chunk()?;
    let mut root = root.clone();
    let mut seen = HashSet::new();
    let mut first = None;
    while seen.insert(root.clone()) {
        let decl = facts.decl_by_id(&root)?;
        if !matches!(decl.kind, crate::DeclKind::Local { is_iter: false, .. })
            || decl.multi_return_index.is_some_and(|index| index > 0)
        {
            return None;
        }
        let value = LuaExpr::cast(decl.value_expr_syntax?.to_node_from_root(chunk.syntax())?)?;
        let value = guards::unparenthesized(value)?;
        let LuaExpr::NameExpr(name) = &value else {
            return None;
        };
        root = model.resolve_name(name.get_position())?;
        first.get_or_insert(value);
        if bindings.iter().any(|(param, _)| *param == root) {
            return first;
        }
    }
    None
}

/// Return possible reaching assignments; `None` denotes the original declaration value.
fn reaching_values(
    model: &SemanticModel,
    root: &SemanticId,
    query: &LuaExpr,
) -> Vec<Option<LuaExpr>> {
    let Some(tree) = model.flow_tree() else {
        return vec![None];
    };
    if tree
        .latest_decl_assignment(root, query.get_position())
        .is_none()
    {
        return vec![None];
    }
    let Some(start) = tree.get_flow_id_at(query.get_position()) else {
        return vec![None];
    };
    let Some(chunk) = model.chunk() else {
        return vec![None];
    };
    let mut pending = vec![start];
    let mut visited = HashSet::new();
    let mut values = Vec::new();
    while let Some(flow) = pending.pop() {
        if !visited.insert(flow) {
            continue;
        }
        let Some(node) = tree.get_flow_node(flow) else {
            values.push(None);
            continue;
        };
        let replacement = tree.get_flow_effects(flow).iter().find_map(|effect| {
            if let FlowEffect::AssignDecl {
                decl, value_syntax, ..
            } = effect
                && decl == root
                && let Some(value) = value_syntax
                    .to_node_from_root(chunk.syntax())
                    .and_then(LuaExpr::cast)
                && value.get_range().end() < query.get_position()
            {
                Some(value)
            } else {
                None
            }
        });
        if let Some(value) = replacement {
            values.push(Some(value));
            continue;
        }
        if matches!(node.kind, FlowNodeKind::Unreachable) {
            continue;
        }
        match &node.antecedent {
            Some(FlowAntecedent::Single(previous)) => pending.push(*previous),
            Some(FlowAntecedent::Multiple(id)) => {
                if let Some(previous) = tree.get_multi_antecedents(*id) {
                    pending.extend(previous.iter().copied());
                } else {
                    values.push(None);
                }
            }
            None => values.push(None),
        }
    }
    values
}

fn write_conflicts(
    model: &SemanticModel,
    expected: &Access,
    var: &LuaVarExpr,
    bindings: &Bindings,
) -> bool {
    let written = match var {
        LuaVarExpr::NameExpr(name) => {
            let Some(root) = model.resolve_name(name.get_position()) else {
                return false;
            };
            // Rebinding a callee parameter or a local alias does not replace the caller's table.
            if bindings.iter().any(|(param, _)| *param == root) {
                return false;
            }
            vec![(root, Vec::new())]
        }
        LuaVarExpr::IndexExpr(index) => access_paths(
            model,
            &LuaExpr::IndexExpr(index.clone()),
            bindings,
            expected,
        ),
    };
    written
        .iter()
        .any(|written| path_write_affects(written, &expected.0, &expected.1))
}

fn local_callees(model: &SemanticModel, call: &LuaCallExpr) -> Vec<LuaClosureExpr> {
    let Some(prefix) = call.get_prefix_expr() else {
        return Vec::new();
    };
    let mut callees = resolve_callees(model, prefix);
    let mut seen = HashSet::new();
    callees.retain(|callee| seen.insert(callee.get_syntax_id()));
    callees
}

fn resolve_callees(model: &SemanticModel, expr: LuaExpr) -> Vec<LuaClosureExpr> {
    let mut pending = vec![expr];
    let mut seen = HashSet::new();
    let mut callees = Vec::new();
    while let Some(expr) = pending.pop() {
        let Some(expr) = guards::unparenthesized(expr) else {
            continue;
        };
        if !seen.insert(expr.get_syntax_id()) {
            continue;
        }
        match &expr {
            LuaExpr::ClosureExpr(closure) => callees.push(closure.clone()),
            LuaExpr::NameExpr(name) => {
                let Some(id @ SemanticId::Decl(_)) = model.resolve_name(name.get_position()) else {
                    continue;
                };
                let SemanticId::Decl(key) = &id else {
                    continue;
                };
                if key.file_id != model.file_id() {
                    continue;
                }
                let (Some(facts), Some(tree), Some(chunk)) =
                    (model.file_facts(), model.flow_tree(), model.chunk())
                else {
                    continue;
                };
                let Some(decl) = facts.decl_by_id(&id) else {
                    continue;
                };
                if tree.has_decl_assignment(&id) {
                    let owner = decl
                        .owner_syntax
                        .and_then(|owner| owner.to_node_from_root(chunk.syntax()));
                    if owner.is_none_or(|owner| {
                        LuaClosureId::from_node(&owner) != LuaClosureId::from_node(name.syntax())
                    }) {
                        continue;
                    }
                }
                for value in reaching_values(model, &id, &expr) {
                    let value = value.or_else(|| {
                        decl.value_expr_syntax
                            .and_then(|value| value.to_node_from_root(chunk.syntax()))
                            .and_then(LuaExpr::cast)
                    });
                    if let Some(value) = value {
                        pending.push(value);
                    }
                }
            }
            _ => {}
        }
    }
    callees
}

fn mutator_conflicts(
    model: &SemanticModel,
    expected: &Access,
    call: &LuaCallExpr,
    bindings: &Bindings,
) -> bool {
    let Some(prefix) = call.get_prefix_expr().and_then(guards::unparenthesized) else {
        return false;
    };
    let LuaExpr::NameExpr(name) = prefix else {
        return false;
    };
    if name.get_name_text().as_deref() != Some("rawset") {
        return false;
    }
    let Some(SemanticId::Decl(decl)) = model.resolve_name(name.get_position()) else {
        return false;
    };
    if !model.is_std_file(decl.file_id) {
        return false;
    }
    let Some(args) = call.get_args_list() else {
        return false;
    };
    let args = args.get_args().collect::<Vec<_>>();
    let Some(receiver) = args.first() else {
        return false;
    };
    let mut written = access_paths(model, receiver, bindings, expected);
    let key = args
        .get(1)
        .map(|key| model.type_of_expr(key.get_syntax_id()));
    let key = match key {
        Some(LuaType::StringConst(key) | LuaType::DocStringConst(key)) => {
            LuaMemberKey::Name((*key).clone())
        }
        Some(LuaType::IntegerConst(key) | LuaType::DocIntegerConst(key)) => {
            LuaMemberKey::Integer(key)
        }
        _ => LuaMemberKey::None,
    };
    written.iter_mut().any(|path| {
        path.1.push(key.clone());
        path_write_affects(path, &expected.0, &expected.1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VirtualWorkspace;

    #[test]
    fn visible_callee_chains_preserve_receiver_identity() {
        for (argument, conflicts) in [("receiver", true), ("other", false)] {
            let mut ws = VirtualWorkspace::new();
            let source = format!(
                r#"
local receiver = {{value = 1}}
local other = {{value = 1}}
local function leaf(target) target.value = nil end
local function middle(target) leaf(target) end
local function outer(target) middle(target) end
outer({argument})
local observed = receiver.value
"#
            );
            let file = ws.def(&source);
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == "receiver.value")
                .unwrap();
            let call = chunk
                .descendants::<LuaCallExpr>()
                .find(|call| call.get_text() == format!("outer({argument})"))
                .unwrap();
            assert_eq!(
                calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
                conflicts
            );
        }
    }
    fn assert_known_call_conflicts(definitions: &str, call_text: &str) {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!("local receiver = {{value = 1}}\n{definitions}\n{call_text}\nlocal observed = receiver.value"));
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == call_text)
            .unwrap();
        assert!(calls_conflict(
            &model,
            &index,
            call.syntax(),
            call.get_range().end()
        ));
    }

    #[test]
    fn deep_known_callees_do_not_hide_conflicting_writes() {
        let mut definitions = "local function step0(target) target.value = nil end\n".to_string();
        for step in 1..2050 {
            definitions.push_str(&format!(
                "local function step{step}(target) step{}(target) end\n",
                step - 1
            ));
        }
        assert_known_call_conflicts(&definitions, "step2049(receiver)");
    }

    #[test]
    fn wide_known_callees_do_not_hide_conflicting_writes() {
        let mut definitions = "local function write(target) target.value = nil end\n".to_string();
        for step in 0..2048 {
            definitions.push_str(&format!(
                "local function harmless{step}(target) return target end\n"
            ));
        }
        definitions.push_str("local function outer(target)\nwrite(target)\n");
        for step in 0..2048 {
            definitions.push_str(&format!("harmless{step}(target)\n"));
        }
        definitions.push_str("end");
        assert_known_call_conflicts(&definitions, "outer(receiver)");
    }

    #[test]
    fn long_callee_aliases_retain_visible_writes() {
        let mut definitions = "local function alias0(target) target.value = nil end\n".to_string();
        for alias in 1..40 {
            definitions.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
        }
        assert_known_call_conflicts(&definitions, "alias39(receiver)");
    }

    #[test]
    fn distant_callee_rebinding_retains_visible_writes() {
        let mut definitions = "local function writer(target) target.value = nil end\nlocal action = function() end\naction = writer\nlocal count = 0\n".to_string();
        for _ in 0..300 {
            definitions.push_str("count = count + 1\n");
        }
        assert_known_call_conflicts(&definitions, "action(receiver)");
    }

    #[test]
    fn long_formal_aliases_retain_visible_writes() {
        let mut definitions = "local other = {}\nlocal function outer(target)\nlocal alias0 = target\ntarget = other\n".to_string();
        for alias in 1..40 {
            definitions.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
        }
        definitions.push_str("alias39.value = nil\nend");
        assert_known_call_conflicts(&definitions, "outer(receiver)");
    }

    #[test]
    fn distant_formal_rebinding_retains_visible_writes() {
        let mut definitions =
            "local other = {}\nlocal function outer(target)\ntarget = receiver\nlocal count = 0\n"
                .to_string();
        for _ in 0..300 {
            definitions.push_str("count = count + 1\n");
        }
        definitions.push_str("target.value = nil\nend");
        assert_known_call_conflicts(&definitions, "outer(other)");
    }

    #[test]
    fn detached_member_captures_do_not_target_the_replacement() {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(
            "local receiver = {child = {value = 1}}\nlocal saved = receiver.child\nreceiver.child = {value = 2}\nlocal function write(target) target.value = nil end\nwrite(saved)\nlocal observed = receiver.child.value",
        );
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.child.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == "write(saved)")
            .unwrap();
        assert!(!calls_conflict(
            &model,
            &index,
            call.syntax(),
            call.get_range().end()
        ));
    }
}
