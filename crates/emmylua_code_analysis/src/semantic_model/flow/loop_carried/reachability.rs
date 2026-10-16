//! Completed per-file CFG reachability, excluding literal-impossible branches.
//! This uses syntax only; type-query incompleteness cannot change the graph.

use super::super::guards;
use crate::semantic_db::flow::{FlowAntecedent, FlowId, FlowNodeKind};
use crate::semantic_model::SemanticModel;
use emmylua_parser::LuaExpr;
use std::rc::Rc;

pub(super) fn reachable_flows(model: &SemanticModel) -> Rc<Vec<bool>> {
    if let Some(reachable) = model
        .cache
        .borrow()
        .loop_statement_reachable
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
        // Start marks each CFG entry, including closure entries. Seed every
        // such node; this does not assume a single entry per file.
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
        .loop_statement_reachable
        .insert(model.file_id(), reachable.clone());
    reachable
}
