//! Reaching values for fixed member callees and captured-member boundaries.

use super::{
    Access, Bindings, CalleeValue, Invocations, SemanticModel, access_paths, guards,
    index_access_path, member_access_path, mutator_conflicts, path_write_affects,
};
use crate::LuaMemberKey;
use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaClosureId};
use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaCallExpr, LuaExpr, LuaFuncStat, LuaVarExpr};
use std::collections::HashSet;

/// Resolve member values along reaching paths, using invocation sites when a
/// callee's flow crosses its lexical entry. Unknown/ancestor writes stop that
/// path rather than reviving an older callable with the same member name.
pub(super) fn member_values(
    model: &SemanticModel,
    target: &Access,
    query: &LuaExpr,
    bindings: &Bindings,
    targets: &[Access],
    invocations: &Invocations,
) -> Vec<CalleeValue> {
    let (Some(tree), Some(chunk)) = (model.flow_tree(), model.chunk()) else {
        return Vec::new();
    };
    let Some(start) = tree.get_flow_id_at(query.get_position()) else {
        return Vec::new();
    };
    let mut pending = vec![(
        start,
        LuaClosureId::from_node(query.syntax()),
        query.get_position(),
        bindings.clone(),
        invocations.clone(),
    )];
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    while let Some((flow, closure, before, bindings, invocations)) = pending.pop() {
        if !seen.insert((flow, closure, before, bindings.clone(), invocations.clone())) {
            continue;
        }
        let Some(node) = tree.get_flow_node(flow) else {
            continue;
        };
        let syntax = node
            .kind
            .assignment_syntax()
            .and_then(|id| id.to_node_from_root(chunk.syntax()))
            .or_else(|| match &node.kind {
                FlowNodeKind::DeclPosition(at) => chunk
                    .syntax()
                    .token_at_offset(*at)
                    .right_biased()
                    .and_then(|token| token.parent()),
                FlowNodeKind::CallExprStat(ptr) => {
                    ptr.to_node(&chunk).map(|node| node.syntax().clone())
                }
                FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                    ptr.to_node(&chunk).map(|node| node.syntax().clone())
                }
                _ => None,
            });
        if matches!(&node.kind, FlowNodeKind::Start)
            || syntax
                .as_ref()
                .is_some_and(|syntax| LuaClosureId::from_node(syntax) != closure)
        {
            if let Some(index) = invocations
                .iter()
                .rposition(|entry| entry.closure == closure)
            {
                let entry = &invocations[index];
                if let Some(caller) = entry.site.to_node_from_root(chunk.syntax())
                    && let Some(caller_flow) = tree.get_flow_id_at(caller.text_range().start())
                {
                    pending.push((
                        caller_flow,
                        LuaClosureId::from_node(&caller),
                        caller.text_range().start(),
                        entry.bindings.as_ref().clone(),
                        invocations[..index].to_vec(),
                    ));
                }
                continue;
            }
            if mutable_captured_member(model, target) {
                continue;
            }
        }
        let mut writes = Vec::new();
        let mut mutates = false;
        if let Some(syntax) = syntax.filter(|syntax| syntax.text_range().end() < before) {
            mutates = syntax
                .descendants()
                .filter_map(LuaCallExpr::cast)
                .filter(|call| LuaClosureId::from_node(call.syntax()) == closure)
                .any(|call| mutator_conflicts(model, target, &call, &bindings));
            if let Some(assign) = LuaAssignStat::cast(syntax.clone()) {
                let (vars, rhs) = assign.get_var_and_expr_list();
                for (index, var) in vars.iter().enumerate() {
                    let paths = match var {
                        LuaVarExpr::NameExpr(name) => model
                            .resolve_name(name.get_position())
                            .map(|id| vec![(id, Vec::new())])
                            .unwrap_or_default(),
                        LuaVarExpr::IndexExpr(index) => access_paths(
                            model,
                            &LuaExpr::IndexExpr(index.clone()),
                            &bindings,
                            targets,
                        ),
                    };
                    for path in paths {
                        writes.push((path, rhs.get(index).cloned()));
                    }
                }
            } else if let Some(func) = LuaFuncStat::cast(syntax) {
                if let Some(var) = func.get_func_name()
                    && let Some(expr) = LuaExpr::cast(var.syntax().clone())
                {
                    for path in access_paths(model, &expr, &bindings, targets) {
                        writes.push((path, func.get_closure().map(LuaExpr::ClosureExpr)));
                    }
                }
            } else if matches!(node.kind, FlowNodeKind::DeclPosition(_)) {
                for effect in tree.get_flow_effects(flow) {
                    if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                        writes.push((
                            (decl.clone(), Vec::new()),
                            value_syntax
                                .to_node_from_root(chunk.syntax())
                                .and_then(LuaExpr::cast),
                        ));
                    }
                }
            }
        }
        let mut replaced = false;
        for (written, value) in writes {
            if path_write_affects(&written, &target.0, &target.1) {
                replaced = true;
                if !written.1.iter().any(LuaMemberKey::is_none)
                    && let Some(value) = value
                        .and_then(|value| table_member_value(value, &target.1[written.1.len()..]))
                {
                    values.push((value, bindings.clone(), invocations.clone()));
                }
            }
        }
        if replaced || mutates || matches!(&node.kind, FlowNodeKind::Unreachable) {
            continue;
        }
        match &node.antecedent {
            Some(FlowAntecedent::Single(previous)) => {
                pending.push((*previous, closure, before, bindings, invocations))
            }
            Some(FlowAntecedent::Multiple(id)) => {
                if let Some(previous) = tree.get_multi_antecedents(*id) {
                    pending.extend(previous.iter().map(|flow| {
                        (
                            *flow,
                            closure,
                            before,
                            bindings.clone(),
                            invocations.clone(),
                        )
                    }));
                }
            }
            None => {}
        }
    }
    values
}

/// Without an invocation, a mutable captured member has no known runtime state.
/// A single fixed definition is usable; replacement/ancestor writes make it opaque.
fn mutable_captured_member(model: &SemanticModel, target: &Access) -> bool {
    let Some(chunk) = model.chunk() else {
        return true;
    };
    if chunk
        .descendants::<LuaCallExpr>()
        .any(|call| mutator_conflicts(model, target, &call, &Bindings::new()))
    {
        return true;
    }
    let mut writes = 0;
    if let Some(decl) = model
        .file_facts()
        .and_then(|facts| facts.decl_by_id(&target.0))
        && let Some(value) = decl
            .value_expr_syntax
            .and_then(|id| id.to_node_from_root(chunk.syntax()))
            .and_then(LuaExpr::cast)
        && table_member_value(value, &target.1).is_some()
    {
        writes += 1;
    }
    let mut vars = Vec::new();
    for assign in chunk.descendants::<LuaAssignStat>() {
        vars.extend(assign.get_var_and_expr_list().0);
    }
    for func in chunk.descendants::<LuaFuncStat>() {
        vars.extend(func.get_func_name());
    }
    for var in vars {
        let Some(expr) = LuaExpr::cast(var.syntax().clone()) else {
            continue;
        };
        let path = match var {
            LuaVarExpr::NameExpr(_) => index_access_path(model, &expr),
            LuaVarExpr::IndexExpr(_) => member_access_path(model, &expr),
        };
        if let Some(path) = path
            && path_write_affects(&path, &target.0, &target.1)
        {
            writes += 1;
            if path.1 != target.1 || writes > 1 {
                return true;
            }
        }
    }
    false
}

pub(super) fn literal_member_key(key: emmylua_parser::LuaIndexKey) -> LuaMemberKey {
    match key {
        emmylua_parser::LuaIndexKey::Name(name) => LuaMemberKey::Name(name.get_name_text().into()),
        emmylua_parser::LuaIndexKey::String(name) => LuaMemberKey::Name(name.get_value().into()),
        emmylua_parser::LuaIndexKey::Integer(number) => match number.get_number_value() {
            emmylua_parser::NumberResult::Int(value) => LuaMemberKey::Integer(value),
            emmylua_parser::NumberResult::Uint(value) => LuaMemberKey::Integer(value as i64),
            emmylua_parser::NumberResult::Float(value) => {
                crate::semantic_db::types::exact_integer(value)
                    .map(LuaMemberKey::Integer)
                    .unwrap_or(LuaMemberKey::None)
            }
            _ => LuaMemberKey::None,
        },
        emmylua_parser::LuaIndexKey::Idx(index) => LuaMemberKey::Integer(index as i64),
        _ => LuaMemberKey::None,
    }
}

fn table_member_value(mut value: LuaExpr, keys: &[LuaMemberKey]) -> Option<LuaExpr> {
    for key in keys {
        let LuaExpr::TableExpr(table) = guards::unparenthesized(value)? else {
            return None;
        };
        let mut selected = None;
        for field in table.get_fields() {
            let field_key = literal_member_key(field.get_field_key()?);
            if field_key == *key {
                selected = field.get_value_expr();
            } else if field_key.is_none() {
                selected = None;
            }
        }
        value = selected?;
    }
    Some(value)
}
