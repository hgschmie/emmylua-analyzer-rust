//! Work-list traversal of visible receiver writes and callees.

use super::{
    Access, Bindings, Invocation, Invocations, SemanticId, SemanticModel, access_paths,
    callee_reachable_flows, guards, local_callees, mutator_conflicts, path_order, receiver_targets,
    write_conflicts,
};
use crate::semantic_db::flow::LuaClosureId;
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaCallExpr, LuaClosureExpr, LuaExpr, LuaFuncStat, LuaSyntaxNode,
    LuaVarExpr,
};
use rowan::TextSize;
use std::{collections::HashSet, rc::Rc};

fn may_be_callee_value(value: LuaExpr) -> bool {
    guards::unparenthesized(value).is_some_and(|value| {
        matches!(
            value,
            LuaExpr::ClosureExpr(_) | LuaExpr::NameExpr(_) | LuaExpr::IndexExpr(_)
        )
    })
}

/// Inspect all direct writes in a visible body before expanding its callees.
/// Repeated callee/binding states terminate recursion. Only receiver prefixes
/// of either the guarded member or a fixed syntax path are retained, so descendant
/// arguments cannot create an unbounded sequence of distinct states.
pub(super) fn scan(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    after: TextSize,
    before: TextSize,
) -> bool {
    let cached = model
        .cache
        .borrow()
        .member_call_resolution
        .get(&model.file_id())
        .copied();
    // This flag reads only syntax from the model's immutable database snapshot.
    // Query incompleteness does not change it; receiver paths below also resolve
    // semantic identities, so their cache publication checks completeness.
    // The root-model lifetime invariant is documented in cache.rs.
    let needs_members = cached.unwrap_or_else(|| {
        let present = model.chunk().is_some_and(|chunk| {
            chunk.descendants::<LuaCallExpr>().any(|call| {
                call.get_prefix_expr()
                    .and_then(guards::unparenthesized)
                    .is_some_and(|prefix| matches!(prefix, LuaExpr::IndexExpr(_)))
            }) || chunk
                .descendants::<LuaFuncStat>()
                .any(|func| matches!(func.get_func_name(), Some(LuaVarExpr::IndexExpr(_))))
                || chunk.descendants::<LuaAssignStat>().any(|assign| {
                    let (vars, values) = assign.get_var_and_expr_list();
                    vars.iter().zip(values).any(|(var, value)| {
                        matches!(var, LuaVarExpr::IndexExpr(_)) && may_be_callee_value(value)
                    })
                })
                || chunk
                    .descendants::<emmylua_parser::LuaTableField>()
                    .any(|field| field.get_value_expr().is_some_and(may_be_callee_value))
        });
        model
            .cache
            .borrow_mut()
            .member_call_resolution
            .insert(model.file_id(), present);
        present
    });
    let targets = if needs_members {
        receiver_targets(model, expected)
    } else {
        Rc::new(vec![expected.clone()])
    };
    let mut bodies = vec![(
        node.clone(),
        after,
        before,
        Bindings::new(),
        Invocations::new(),
        false,
    )];
    let reachable = callee_reachable_flows(model);
    let tree = model.flow_tree();
    let mut seen = HashSet::new();
    while let Some((body, after, before, bindings, invocations, writes)) = bodies.pop() {
        let mut pending = vec![body];
        let mut calls = Vec::new();
        while let Some(node) = pending.pop() {
            if node.text_range().start() >= before
                || node.text_range().end() <= after
                || LuaClosureExpr::can_cast(node.kind().into())
            {
                continue;
            }
            // Prune only bound statements/expressions. Missing bindings do not
            // prove unreachability and must retain the visible-effect scan.
            if (LuaAssignStat::can_cast(node.kind().into())
                || LuaFuncStat::can_cast(node.kind().into())
                || LuaCallExpr::can_cast(node.kind().into()))
                && tree
                    .as_ref()
                    .and_then(|tree| tree.get_flow_id_at(node.text_range().start()))
                    .and_then(|flow| reachable.get(flow.0 as usize))
                    == Some(&false)
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
            if writes
                && node.text_range().end() <= before
                && let Some(func) = LuaFuncStat::cast(node.clone())
                && let Some(var) = func.get_func_name()
                && write_conflicts(model, expected, &var, &bindings)
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
            for closure in local_callees(model, &call, &bindings, &targets, &invocations) {
                let Some(body) = closure.get_block() else {
                    continue;
                };
                let mut nested = bindings.clone();
                let mut args = call
                    .get_args_list()
                    .map(|args| args.get_args().collect::<Vec<_>>())
                    .unwrap_or_default();
                if call.is_colon_call()
                    && let Some(LuaExpr::IndexExpr(index)) = call.get_prefix_expr()
                    && let Some(receiver) = index.get_prefix_expr()
                {
                    args.insert(0, receiver);
                }
                let mut formals = Vec::new();
                if let Some(facts) = model.file_facts() {
                    formals.extend(
                        facts
                            .decls_by_owner_syntax(closure.get_syntax_id())
                            .filter(|decl| decl.is_implicit_self)
                            .map(|decl| decl.id.clone()),
                    );
                }
                if let Some(params) = closure.get_params_list() {
                    formals.extend(
                        params
                            .get_params()
                            .map(|param| SemanticId::decl(model.file_id(), param.get_range())),
                    );
                }
                for (index, id) in formals.into_iter().enumerate() {
                    let mut paths = args.get(index).map_or_else(Vec::new, |arg| {
                        access_paths(model, arg, &bindings, &targets)
                    });
                    path_order::canonicalize(&mut paths);
                    nested.retain(|(bound, _)| *bound != id);
                    // Empty bindings identify omitted/scalar actuals and must not
                    // revive a previous recursive invocation's receiver.
                    nested.push((id, paths));
                }
                // These are same-file formal declarations, identified by position.
                nested.sort_by_key(|(id, _)| match id {
                    SemanticId::Decl(key) => Some(key.name_range.start()),
                    _ => None,
                });
                let mut nested_invocations = invocations.clone();
                let closure_id = LuaClosureId::from_node(closure.syntax());
                if needs_members {
                    let edge = Invocation::new(closure_id, call.get_syntax_id(), &bindings);
                    nested_invocations.retain(|entry| *entry != edge);
                    nested_invocations.push(edge);
                }
                if seen.insert((
                    closure.get_syntax_id(),
                    nested.clone(),
                    nested_invocations.clone(),
                )) {
                    bodies.push((
                        body.syntax().clone(),
                        TextSize::from(0),
                        body.get_range().end(),
                        nested,
                        nested_invocations,
                        true,
                    ));
                }
            }
        }
    }
    false
}
