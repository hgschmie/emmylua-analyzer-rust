//! Visible call effects for member guards. Opaque calls preserve existing guards.
//! This bounded traversal detects possible writes; it never propagates callee values.

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
    let expr = LuaExpr::IndexExpr(index.clone());
    let Some(expected) =
        stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
    else {
        return false;
    };
    scan(
        model,
        &expected,
        node,
        before,
        &Vec::new(),
        false,
        0,
        &mut 2048,
    )
}

#[allow(clippy::too_many_arguments)]
fn scan(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    before: TextSize,
    bindings: &Bindings,
    writes: bool,
    depth: usize,
    budget: &mut usize,
) -> bool {
    if depth >= 8 {
        return false;
    }
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        if node.text_range().start() >= before || LuaClosureExpr::can_cast(node.kind().into()) {
            continue;
        }
        if writes
            && node.text_range().end() <= before
            && let Some(assign) = LuaAssignStat::cast(node.clone())
            && assign
                .get_var_and_expr_list()
                .0
                .iter()
                .any(|var| write_conflicts(model, expected, var, bindings))
        {
            return true;
        }
        if node.text_range().end() <= before
            && let Some(call) = LuaCallExpr::cast(node.clone())
        {
            if mutator_conflicts(model, expected, &call, bindings) {
                return true;
            }
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
                    for (param, arg) in params.get_params().zip(args.iter()) {
                        let id = SemanticId::decl(model.file_id(), param.get_range());
                        let paths = access_paths(model, arg, bindings, 0, &mut 256);
                        if !paths.is_empty() {
                            nested.retain(|(bound, _)| *bound != id);
                            nested.push((id, paths));
                        }
                    }
                }
                if scan(
                    model,
                    expected,
                    body.syntax(),
                    body.get_range().end(),
                    &nested,
                    true,
                    depth + 1,
                    budget,
                ) {
                    return true;
                }
            }
        }
        pending.extend(node.children());
    }
    false
}

fn access_paths(
    model: &SemanticModel,
    expr: &LuaExpr,
    bindings: &Bindings,
    depth: usize,
    budget: &mut usize,
) -> Vec<Access> {
    let Some((root, keys)) =
        stable_access_path(model, expr).or_else(|| index_access_path(model, expr))
    else {
        return Vec::new();
    };
    let Some((_, initial)) = bindings.iter().find(|(param, _)| *param == root) else {
        let Some(mut paths) = formal_alias_paths(model, expr, &root, bindings, depth, budget)
        else {
            return vec![(root, keys)];
        };
        for (_, prefix) in &mut paths {
            prefix.extend(keys.clone());
        }
        return paths;
    };
    let mut paths = parameter_paths(model, expr, &root, initial, bindings, depth, budget);
    for (_, prefix) in &mut paths {
        prefix.extend(keys.clone());
    }
    paths
}

/// Resolve name-alias captures that lead to bound formals without reviving detached member paths.
fn formal_alias_paths(
    model: &SemanticModel,
    expr: &LuaExpr,
    root: &SemanticId,
    bindings: &Bindings,
    depth: usize,
    budget: &mut usize,
) -> Option<Vec<Access>> {
    if depth >= 8 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    let facts = model.file_facts()?;
    let chunk = model.chunk()?;
    let decl = facts.decl_by_id(root)?;
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
    let source = model.resolve_name(name.get_position())?;
    let initial = if bindings.iter().any(|(param, _)| *param == source) {
        access_paths(model, &value, bindings, depth + 1, budget)
    } else {
        formal_alias_paths(model, &value, &source, bindings, depth + 1, budget)?
    };
    Some(parameter_paths(
        model,
        expr,
        root,
        &initial,
        bindings,
        depth + 1,
        budget,
    ))
}

/// Keep each possible actual receiver until a preceding assignment replaces it on that path.
fn parameter_paths(
    model: &SemanticModel,
    expr: &LuaExpr,
    root: &SemanticId,
    initial: &[Access],
    bindings: &Bindings,
    depth: usize,
    budget: &mut usize,
) -> Vec<Access> {
    if depth >= 8 {
        return initial.to_vec();
    }
    let mut paths = Vec::new();
    for value in reaching_values(model, root, expr, budget) {
        match value {
            Some(value) => paths.extend(access_paths(model, &value, bindings, depth + 1, budget)),
            None => paths.extend_from_slice(initial),
        }
    }
    paths
}

/// Return possible reaching assignments; `None` denotes the original declaration value.
fn reaching_values(
    model: &SemanticModel,
    root: &SemanticId,
    query: &LuaExpr,
    budget: &mut usize,
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
        if *budget == 0 {
            values.push(None);
            break;
        }
        *budget -= 1;
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
            0,
            &mut 256,
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
    let mut callees = resolve_callees(model, prefix, 0, &mut 256);
    let mut seen = HashSet::new();
    callees.retain(|callee| seen.insert(callee.get_syntax_id()));
    callees
}

fn resolve_callees(
    model: &SemanticModel,
    expr: LuaExpr,
    depth: usize,
    budget: &mut usize,
) -> Vec<LuaClosureExpr> {
    if depth >= 8 || *budget == 0 {
        return Vec::new();
    }
    *budget -= 1;
    let Some(expr) = guards::unparenthesized(expr) else {
        return Vec::new();
    };
    match &expr {
        LuaExpr::ClosureExpr(closure) => vec![closure.clone()],
        LuaExpr::NameExpr(name) => {
            let Some(id @ SemanticId::Decl(_)) = model.resolve_name(name.get_position()) else {
                return Vec::new();
            };
            let SemanticId::Decl(key) = &id else {
                return Vec::new();
            };
            if key.file_id != model.file_id() {
                return Vec::new();
            }
            let (Some(facts), Some(tree), Some(chunk)) =
                (model.file_facts(), model.flow_tree(), model.chunk())
            else {
                return Vec::new();
            };
            let Some(decl) = facts.decl_by_id(&id) else {
                return Vec::new();
            };
            if tree.has_decl_assignment(&id) {
                let owner = decl
                    .owner_syntax
                    .and_then(|owner| owner.to_node_from_root(chunk.syntax()));
                if owner.is_none_or(|owner| {
                    LuaClosureId::from_node(&owner) != LuaClosureId::from_node(name.syntax())
                }) {
                    return Vec::new();
                }
            }
            let mut callees = Vec::new();
            for value in reaching_values(model, &id, &expr, budget) {
                let value = value.or_else(|| {
                    decl.value_expr_syntax
                        .and_then(|value| value.to_node_from_root(chunk.syntax()))
                        .and_then(LuaExpr::cast)
                });
                if let Some(value) = value {
                    callees.extend(resolve_callees(model, value, depth + 1, budget));
                }
            }
            callees
        }
        _ => Vec::new(),
    }
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
    let mut written = access_paths(model, receiver, bindings, 0, &mut 256);
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
