//! Prior iterations can invalidate member facts established before a loop.

use std::collections::HashSet;

use super::super::{MemberWrite, member_write_match};
use super::{SemanticId, SemanticModel, callee_reachable_flows, member_access_path};
use crate::semantic_db::flow::{
    FlowAntecedent, FlowId, FlowNode, FlowNodeKind, FlowTree, LuaClosureId,
};
use emmylua_parser::{LuaAstNode, LuaClosureExpr, LuaExpr, LuaIndexExpr, LuaLoopStat};
use rowan::TextRange;

/// Return loops whose prior iteration may replace this member. Fresh assignments
/// and guards within the current iteration are consumed before these boundaries.
pub(in crate::semantic_model::flow) fn write_boundaries(
    model: &SemanticModel,
    tree: &FlowTree,
    member: &SemanticId,
    access: Option<&LuaIndexExpr>,
) -> Vec<TextRange> {
    let (Some(access), Some(chunk)) = (access, model.chunk()) else {
        return Vec::new();
    };
    let loops = access
        .syntax()
        .ancestors()
        .take_while(|node| !LuaClosureExpr::can_cast(node.kind().into()))
        .filter_map(LuaLoopStat::cast)
        .collect::<Vec<_>>();
    if loops.is_empty() {
        return Vec::new();
    }
    let receiver = member_access_path(model, &LuaExpr::IndexExpr(access.clone()));
    let closure = LuaClosureId::from_node(access.syntax());
    let reachable = callee_reachable_flows(model);
    loops
        .into_iter()
        .filter_map(|loop_stat| {
            let range = loop_stat.get_range();
            let reentries = tree.get_loop_reentries(loop_stat.get_syntax_id())?;
            let repeated = repeating_flows(tree, reentries.flows(), &reachable, range);
            if repeated.is_empty() {
                return None;
            }
            let in_iteration = match &loop_stat {
                LuaLoopStat::ForStat(stat) => stat
                    .get_block()
                    .is_some_and(|body| body.get_range().contains_range(access.get_range())),
                LuaLoopStat::ForRangeStat(stat) => stat
                    .get_block()
                    .is_some_and(|body| body.get_range().contains_range(access.get_range())),
                _ => true,
            };
            if !in_iteration {
                return None;
            }
            // A newly constructed local receiver does not carry an earlier
            // iteration's member state. Stable aliases already resolve outward.
            if receiver
                .as_ref()
                .and_then(|(root, _)| model.file_facts()?.decl_by_id(root))
                .is_some_and(|decl| {
                    matches!(decl.kind, crate::DeclKind::Local { .. })
                        && range.contains_range(decl.name_range)
                })
            {
                return None;
            }
            let conflict = repeated.into_iter().any(|flow| {
                let Some(node) = tree.get_flow_node(flow) else {
                    return false;
                };
                let syntax = match &node.kind {
                    FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                        Some(ptr.get_syntax_id())
                    }
                    FlowNodeKind::CallExprStat(ptr) => Some(ptr.get_syntax_id()),
                    _ => node.kind.assignment_syntax(),
                };
                let source = syntax
                    .and_then(|id| id.to_node_from_root(chunk.syntax()))
                    .or_else(|| match node.kind {
                        FlowNodeKind::DeclPosition(at) => chunk
                            .syntax()
                            .token_at_offset(at)
                            .right_biased()
                            .and_then(|token| token.parent()),
                        _ => None,
                    });
                let same_iteration = source.is_some_and(|node| {
                    range.contains_range(node.text_range())
                        && LuaClosureId::from_node(&node) == closure
                });
                same_iteration
                    && (member_write_match(model, tree, flow, member, Some(access), None)
                        != MemberWrite::Unrelated
                        || super::super::member_flow_calls_conflict(
                            model,
                            tree,
                            flow,
                            Some(access),
                            Some(range.end()),
                        ))
            });
            conflict.then_some(range)
        })
        .collect()
}

pub(in crate::semantic_model::flow) fn crosses_boundary(
    node: &FlowNode,
    boundaries: &[TextRange],
) -> bool {
    let position = match &node.kind {
        FlowNodeKind::DeclPosition(at) => Some(*at),
        FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
            Some(ptr.get_syntax_id().get_range().start())
        }
        FlowNodeKind::CallExprStat(ptr) => Some(ptr.get_syntax_id().get_range().start()),
        _ => node
            .kind
            .assignment_syntax()
            .map(|syntax| syntax.get_range().start()),
    };
    !boundaries.is_empty()
        && (matches!(node.kind, FlowNodeKind::Start)
            || position.is_some_and(|at| boundaries.iter().any(|range| at < range.start())))
}

/// Only paths reaching normal completion or an explicit continue can repeat.
/// A write on a returning/breaking branch must not erase the next entry's guard.
pub(super) fn repeating_flows(
    tree: &FlowTree,
    entries: [FlowId; 2],
    reachable: &[bool],
    range: TextRange,
) -> HashSet<FlowId> {
    let mut pending = entries.to_vec();
    let mut seen = HashSet::new();
    while let Some(flow) = pending.pop() {
        if !reachable.get(flow.0 as usize).copied().unwrap_or(false) || seen.contains(&flow) {
            continue;
        }
        let Some(node) = tree.get_flow_node(flow) else {
            continue;
        };
        if crosses_boundary(node, std::slice::from_ref(&range)) {
            continue;
        }
        seen.insert(flow);
        match &node.antecedent {
            Some(FlowAntecedent::Single(previous)) => pending.push(*previous),
            Some(FlowAntecedent::Multiple(id)) => {
                pending.extend(tree.get_multi_antecedents(*id).unwrap_or_default())
            }
            None => {}
        }
    }
    seen
}
