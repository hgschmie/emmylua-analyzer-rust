//! Reaching global writes can replace names still resolved to stdlib.
//!
//! A nested-body query has no particular invocation context. Enclosing writes
//! therefore remain possible across all calls to that body, including escaped
//! callbacks. This conservative summary can retain warnings for a known call
//! before a later replacement; lexical definition position cannot prove safety.

use super::reaching_values::{callee_reachable_flows, reaching_values};
use crate::semantic_db::{
    def::{DeclKind, SemanticId},
    flow::LuaClosureId,
};
use crate::semantic_model::SemanticModel;
use emmylua_parser::{
    LuaAstNode, LuaCallExpr, LuaExpr, LuaLoopStat, LuaNameExpr, LuaStat, LuaSyntaxId, LuaVarExpr,
};
use std::{collections::HashSet, rc::Rc};

mod environment;

pub(crate) struct GlobalNameValues {
    pub(crate) has_replacement: bool,
    pub(crate) may_keep_builtin: bool,
    pub(crate) replacements: Vec<LuaExpr>,
}

/// Keep the builtin possible unless a single definition reaches every path.
/// Possible replacements also expose their visible bodies to call-effect scanning.
pub(crate) fn global_name_values(model: &SemanticModel, name: &LuaNameExpr) -> GlobalNameValues {
    let mut result = GlobalNameValues {
        has_replacement: false,
        may_keep_builtin: true,
        replacements: Vec::new(),
    };
    let (Some(text), Some(chunk), Some(facts)) =
        (name.get_name_text(), model.chunk(), model.file_facts())
    else {
        return result;
    };
    let key = (model.file_id(), text.as_str().into());
    let cached = model
        .cache
        .borrow()
        .member_global_name_writes
        .get(&key)
        .cloned();
    let definitions = cached.unwrap_or_else(|| {
        let syntax = Rc::new(
            chunk
                .descendants::<LuaStat>()
                .filter(|statement| {
                    written_name(statement, &text).is_some()
                        || environment::written_values(model, statement, &text).is_some()
                })
                .map(|statement| statement.get_syntax_id())
                .collect::<Vec<_>>(),
        );
        model
            .cache
            .borrow_mut()
            .member_global_name_writes
            .insert(key, syntax.clone());
        syntax
    });
    if definitions.is_empty() {
        return result;
    }
    let query_closure = LuaClosureId::from_node(name.syntax());
    let called = visible_call_closures(model, name);
    let reachable = callee_reachable_flows(model);
    let statements = definitions
        .iter()
        .filter_map(|syntax| {
            syntax
                .to_node_from_root(chunk.syntax())
                .and_then(LuaStat::cast)
        })
        .collect::<Vec<_>>();
    let previous = previous_iteration_nodes(model, name, &statements, &reachable);
    for statement in statements {
        if model
            .flow_tree()
            .and_then(|tree| tree.get_flow_id_at(statement.get_position()))
            .and_then(|flow| reachable.get(flow.0 as usize))
            == Some(&false)
        {
            continue;
        }
        let definition_closure = LuaClosureId::from_node(statement.syntax());
        if let Some(values) = environment::written_values(model, &statement, &text) {
            let same_iteration = statement.get_range().end() <= name.get_position();
            let prior_iteration = previous.contains(&statement.get_syntax_id());
            let can_precede = if definition_closure == query_closure {
                same_iteration || prior_iteration
            } else {
                definition_closure
                    .get_range()
                    .contains_range(query_closure.get_range())
                    || called.contains(&definition_closure)
            };
            if can_precede {
                // Even a missing RHS replaces the slot with nil. The builtin
                // remains a possible callee unless a definite replacement is
                // established; known RHS bodies are also scanned for effects.
                result.has_replacement = true;
                result.replacements.extend(values);
            }
            continue;
        }
        let Some(target) = written_name(&statement, &text) else {
            continue;
        };
        let id = SemanticId::decl(model.file_id(), target.get_range());
        if !facts
            .decl_by_id(&id)
            .is_some_and(|decl| matches!(decl.kind, DeclKind::Global))
        {
            continue;
        }
        if definition_closure == query_closure {
            let values = reaching_values(model, &id, &LuaExpr::NameExpr(name.clone()));
            if !values.is_empty() && values.iter().all(Option::is_some) {
                result.may_keep_builtin = false;
            }
            result.replacements.extend(values.into_iter().flatten());
        } else if definition_closure
            .get_range()
            .contains_range(query_closure.get_range())
            || called.contains(&definition_closure)
        {
            // Invocation order, not definition position, decides captured global
            // state. Enclosing writes and writes in visible called siblings are
            // possible replacements; they do not prove the builtin impossible.
            result.replacements.extend(
                facts
                    .decl_by_id(&id)
                    .and_then(|decl| decl.value_expr_syntax)
                    .and_then(|syntax| syntax.to_node_from_root(chunk.syntax()))
                    .and_then(LuaExpr::cast),
            );
        }
    }
    result.has_replacement |= !result.replacements.is_empty();
    result
}

/// Follow visible same-file name callees without invoking member/type inference.
/// Opaque or member-installed callables retain the normal value-propagation limit.
fn visible_call_closures(model: &SemanticModel, query: &LuaNameExpr) -> HashSet<LuaClosureId> {
    let Some(chunk) = model.chunk() else {
        return HashSet::new();
    };
    let query_closure = LuaClosureId::from_node(query.syntax());
    let calls: Vec<_> = chunk.descendants::<LuaCallExpr>().collect();
    let reachable = callee_reachable_flows(model);
    let previous_calls = previous_iteration_nodes(model, query, &calls, &reachable);
    let can_run = |call: &LuaCallExpr| {
        model
            .flow_tree()
            .and_then(|tree| tree.get_flow_id_at(call.get_position()))
            .and_then(|flow| reachable.get(flow.0 as usize))
            != Some(&false)
    };
    let mut pending: Vec<_> = calls
        .iter()
        .filter(|call| {
            let owner = LuaClosureId::from_node(call.syntax());
            can_run(call)
                && if owner == query_closure {
                    call.get_range().end() <= query.get_position()
                        || previous_calls.contains(&call.get_syntax_id())
                } else {
                    owner.get_range().contains_range(query_closure.get_range())
                }
        })
        .filter_map(LuaCallExpr::get_prefix_expr)
        .collect();
    let mut seen = HashSet::new();
    let mut called = HashSet::new();
    while let Some(expr) = pending.pop() {
        let Some(expr) = super::guards::unparenthesized(expr) else {
            continue;
        };
        if !seen.insert(expr.get_syntax_id()) {
            continue;
        }
        match expr {
            LuaExpr::ClosureExpr(closure) => {
                let id = LuaClosureId::from_node(closure.syntax());
                if called.insert(id) {
                    pending.extend(
                        calls
                            .iter()
                            .filter(|call| {
                                can_run(call) && LuaClosureId::from_node(call.syntax()) == id
                            })
                            .filter_map(LuaCallExpr::get_prefix_expr),
                    );
                }
            }
            LuaExpr::NameExpr(ref name) => {
                // Do not recurse through global_name_values for standard names.
                if matches!(model.resolve_name(name.get_position()), Some(SemanticId::Decl(key)) if key.file_id == model.file_id())
                {
                    pending.extend(super::local_name_values(model, &expr));
                }
            }
            _ => {}
        }
    }
    called
}

/// Nodes after a query may already have executed in a previous iteration.
/// Only paths reaching normal completion or continue contribute; terminating
/// branches and one-time for-loop header expressions cannot create this effect.
fn previous_iteration_nodes<N: LuaAstNode>(
    model: &SemanticModel,
    query: &LuaNameExpr,
    nodes: &[N],
    reachable: &[bool],
) -> HashSet<LuaSyntaxId> {
    let Some(tree) = model.flow_tree() else {
        return HashSet::new();
    };
    let closure = LuaClosureId::from_node(query.syntax());
    let mut previous = HashSet::new();
    for loop_stat in query.ancestors::<LuaLoopStat>() {
        if LuaClosureId::from_node(loop_stat.syntax()) != closure {
            break;
        }
        let in_iteration = match &loop_stat {
            LuaLoopStat::ForStat(stat) => stat
                .get_block()
                .is_some_and(|body| body.get_range().contains_range(query.get_range())),
            LuaLoopStat::ForRangeStat(stat) => stat
                .get_block()
                .is_some_and(|body| body.get_range().contains_range(query.get_range())),
            _ => true,
        };
        if !in_iteration {
            continue;
        }
        let Some(reentries) = tree.get_loop_reentries(loop_stat.get_syntax_id()) else {
            continue;
        };
        let range = loop_stat.get_range();
        let flows =
            super::loop_members::repeating_flows(&tree, reentries.flows(), reachable, range);
        previous.extend(
            nodes
                .iter()
                .filter(|node| {
                    range.contains_range(node.get_range())
                        && LuaClosureId::from_node(node.syntax()) == closure
                        && tree
                            .get_flow_id_at(node.get_position())
                            .is_some_and(|flow| flows.contains(&flow))
                })
                .map(LuaAstNode::get_syntax_id),
        );
    }
    previous
}

fn written_name(statement: &LuaStat, name: &str) -> Option<LuaNameExpr> {
    let vars = match statement {
        LuaStat::AssignStat(assignment) => assignment.get_var_and_expr_list().0,
        LuaStat::FuncStat(function) => function.get_func_name().into_iter().collect(),
        _ => return None,
    };
    vars.into_iter().find_map(|var| match var {
        LuaVarExpr::NameExpr(target) if target.get_name_text().as_deref() == Some(name) => {
            Some(target)
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_builtin_writes_follow_invocation_order() {
        for (source, replaced) in [
            (
                "local function query(value) return type(value) end; type=function() return 'string' end; query(nil)",
                true,
            ),
            (
                "local function replace() type=function() return 'string' end end; replace(); local result=type(nil)",
                true,
            ),
            (
                "local function replace() type=function() return 'string' end end; local alias=replace; local function wrapper() alias() end; wrapper(); local result=type(nil)",
                true,
            ),
            (
                "local function replace() if false then type=function() return 'string' end end end; replace(); local result=type(nil)",
                false,
            ),
            (
                "local function replace() type=function() return 'string' end end; local result=type(nil)",
                false,
            ),
            (
                "local result=type(nil); type=function() return 'string' end",
                false,
            ),
            (
                "local function replace() local type=function() return 'string' end end; replace(); local result=type(nil)",
                false,
            ),
        ] {
            let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
            let file = ws.def(source);
            let model = ws.analysis.semantic_model(file);
            let name = model
                .chunk()
                .unwrap()
                .descendants::<LuaNameExpr>()
                .find(|name| {
                    name.get_name_text().as_deref() == Some("type")
                        && name
                            .syntax()
                            .parent()
                            .is_some_and(|p| LuaCallExpr::can_cast(p.kind().into()))
                })
                .unwrap();
            assert_eq!(
                !global_name_values(&model, &name).replacements.is_empty(),
                replaced,
                "{source}"
            );
        }
    }
}
