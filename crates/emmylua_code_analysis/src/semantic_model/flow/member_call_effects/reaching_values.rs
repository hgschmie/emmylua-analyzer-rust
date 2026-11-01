//! Reachable local-callee assignments, including prior loop iterations.

use std::{collections::HashSet, rc::Rc};

use emmylua_parser::{LuaAstNode, LuaClosureExpr, LuaExpr, LuaLoopStat};
use rowan::TextRange;

use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowId, FlowNodeKind};
use crate::semantic_model::SemanticModel;

use super::guards;

/// Prune literal-impossible paths before accepting a replacement. Backtracking
/// stops at the replacement itself, so checking only later predecessor visits
/// would accept a write before ever inspecting its enclosing false condition.
// FlowTree is fully built before the immutable SemanticModel snapshot is
// exposed. Type-query incompleteness cannot add nodes or antecedents to it.
fn callee_reachable_flows(model: &SemanticModel) -> Rc<Vec<bool>> {
    if let Some(reachable) = model
        .cache
        .borrow()
        .member_callee_reachable
        .get(&model.file_id())
    {
        return reachable.clone();
    }
    let (Some(tree), Some(chunk)) = (model.flow_tree(), model.chunk()) else {
        return Rc::new(Vec::new());
    };
    let count = tree.node_count() as usize;
    let mut allowed = vec![true; count];
    let mut successors = vec![Vec::new(); count];
    let mut pending = Vec::new();
    for (index, allowed_node) in allowed.iter_mut().enumerate() {
        let Some(node) = tree.get_flow_node(FlowId(index as u32)) else {
            continue;
        };
        if matches!(&node.kind, FlowNodeKind::Unreachable) {
            *allowed_node = false;
        }
        if let FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) = &node.kind
            && let Some(condition) = ptr.to_node(&chunk)
            && let Some((LuaExpr::LiteralExpr(literal), truthy)) = guards::normalize_condition(
                condition,
                matches!(node.kind, FlowNodeKind::TrueCondition(_)),
            )
        {
            let actual = match literal.get_literal() {
                Some(emmylua_parser::LuaLiteralToken::Bool(value)) => Some(value.is_true()),
                Some(emmylua_parser::LuaLiteralToken::Nil(_)) => Some(false),
                Some(
                    emmylua_parser::LuaLiteralToken::String(_)
                    | emmylua_parser::LuaLiteralToken::Number(_),
                ) => Some(true),
                _ => None,
            };
            *allowed_node = actual.is_none_or(|actual| actual == truthy);
        }
        let previous = match &node.antecedent {
            Some(FlowAntecedent::Single(previous)) => std::slice::from_ref(previous),
            Some(FlowAntecedent::Multiple(id)) => {
                tree.get_multi_antecedents(*id).unwrap_or_default()
            }
            None => &[],
        };
        for previous in previous {
            if let Some(next) = successors.get_mut(previous.0 as usize) {
                next.push(index);
            }
        }
        if matches!(&node.kind, FlowNodeKind::Start) {
            pending.push(index);
        }
    }
    let mut reachable = vec![false; count];
    while let Some(index) = pending.pop() {
        if allowed[index] && !reachable[index] {
            reachable[index] = true;
            pending.extend(successors[index].iter().copied());
        }
    }
    let reachable = Rc::new(reachable);
    model
        .cache
        .borrow_mut()
        .member_callee_reachable
        .insert(model.file_id(), reachable.clone());
    reachable
}

/// Return possible reaching assignments in the current file. `None` retains the
/// original declaration value or bound formal when no replacement reaches it.
pub(super) fn reaching_values(
    model: &SemanticModel,
    root: &SemanticId,
    query: &LuaExpr,
) -> Vec<Option<LuaExpr>> {
    let Some(tree) = model.flow_tree() else {
        return vec![None];
    };
    if !tree.has_decl_assignment(root) {
        return vec![None];
    }
    let Some(start) = tree.get_flow_id_at(query.get_position()) else {
        return vec![None];
    };
    let Some(chunk) = model.chunk() else {
        return vec![None];
    };
    let reachable = callee_reachable_flows(model);
    let loops = enclosing_loop_reentries(model, root, query);
    let mut entered = HashSet::new();
    let mut pending = vec![start];
    let mut visited = HashSet::new();
    let mut values = Vec::new();
    while let Some(flow) = pending.pop() {
        if !reachable.get(flow.0 as usize).copied().unwrap_or(false) || !visited.insert(flow) {
            continue;
        }
        let Some(node) = tree.get_flow_node(flow) else {
            values.push(None);
            continue;
        };
        // The CFG intentionally omits normal loop backedges. Add them only for
        // this reaching-value query, when a path exits the current iteration.
        // A replacement inside the iteration stops before reaching this boundary.
        let position = match &node.kind {
            FlowNodeKind::DeclPosition(at) => Some(*at),
            FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                Some(ptr.get_syntax_id().get_range().start())
            }
            _ => node
                .kind
                .assignment_syntax()
                .map(|id| id.get_range().start()),
        };
        for (range, reentries) in &loops {
            if (matches!(&node.kind, FlowNodeKind::Start)
                || position.is_some_and(|position| position < range.start()))
                && entered.insert(*range)
            {
                pending.extend(reentries);
            }
        }
        let replacement = tree.get_flow_effects(flow).iter().find_map(|effect| {
            if let FlowEffect::AssignDecl {
                decl, value_syntax, ..
            } = effect
                && decl == root
                && let Some(value) = value_syntax
                    .to_node_from_root(chunk.syntax())
                    .and_then(LuaExpr::cast)
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
        if matches!(&node.kind, FlowNodeKind::Unreachable) {
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

/// Locals declared in a loop are recreated on each iteration. Only enclosing
/// loops in the same closure can feed an older declaration back into this query.
fn enclosing_loop_reentries(
    model: &SemanticModel,
    root: &SemanticId,
    query: &LuaExpr,
) -> Vec<(TextRange, [FlowId; 2])> {
    let (Some(facts), Some(tree)) = (model.file_facts(), model.flow_tree()) else {
        return Vec::new();
    };
    let Some(decl) = facts.decl_by_id(root) else {
        return Vec::new();
    };
    query
        .syntax()
        .ancestors()
        .take_while(|node| LuaClosureExpr::cast(node.clone()).is_none())
        .filter_map(LuaLoopStat::cast)
        .filter(|loop_stat| !loop_stat.get_range().contains_range(decl.name_range))
        .filter(|loop_stat| match loop_stat {
            LuaLoopStat::ForStat(stat) => stat
                .get_block()
                .is_some_and(|block| block.get_range().contains_range(query.get_range())),
            LuaLoopStat::ForRangeStat(stat) => stat
                .get_block()
                .is_some_and(|block| block.get_range().contains_range(query.get_range())),
            // While/repeat conditions run again after each body iteration.
            _ => true,
        })
        .filter_map(|loop_stat| {
            tree.loop_reentries(loop_stat.get_syntax_id())
                .map(|flows| (loop_stat.get_range(), *flows))
        })
        .collect()
}
