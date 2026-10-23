//! Reaching writes for computed reads of inferred table literals.

use std::collections::HashSet;

use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaExpr, LuaIndexExpr, LuaIndexKey, LuaKind, LuaSyntaxId,
    LuaSyntaxKind, LuaTableExpr, LuaVarExpr,
};

use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaClosureId};
use crate::{LuaType, SemanticId};

use super::super::SemanticModel;
use super::super::type_eval::expand_alias_head;

pub(super) fn receiver_type_at(model: &SemanticModel, key: LuaSyntaxId) -> Option<LuaType> {
    let root = model.syntax_tree()?.get_red_root();
    let index = key
        .to_node_from_root(&root)?
        .ancestors()
        .find_map(LuaIndexExpr::cast)?;
    let prefix = index.get_prefix_expr()?;
    let ty = model.type_of_expr_at(prefix.get_syntax_id(), prefix.get_position());
    matches!(ty, LuaType::TableConst(_)).then_some(ty)
}

/// Include reachable string-key writes without converting a fresh table into an open dictionary.
pub(super) fn read_type(
    model: &SemanticModel,
    receiver: &LuaType,
    key_syntax: LuaSyntaxId,
    key_type: &LuaType,
    initial: &LuaType,
) -> Option<LuaType> {
    let LuaType::TableConst(table) = receiver else {
        return None;
    };
    if table.file_id != model.file_id() || !may_be_string(model, key_type) {
        return None;
    }
    let root = model.syntax_tree()?.get_red_root();
    let initial = if initial.is_unknown()
        && LuaSyntaxId::new(LuaKind::Syntax(LuaSyntaxKind::TableEmptyExpr), table.value)
            .to_node_from_root(&root)
            .and_then(LuaTableExpr::cast)
            .is_some_and(|literal| literal.get_fields().next().is_none())
    {
        &LuaType::Nil
    } else {
        initial
    };
    let key = LuaExpr::cast(key_syntax.to_node_from_root(&root)?)?;
    let key_decl = name_decl(model, &key);
    let index = key.syntax().ancestors().find_map(LuaIndexExpr::cast)?;
    let receiver_decl = name_decl(model, &index.get_prefix_expr()?);
    let offset = key_syntax.get_range().start();
    let flow = model.flow_tree()?;
    let start = flow.get_flow_id_at(offset)?;
    let chunk = model.chunk()?;
    let mut pending = vec![(start, true)];
    let mut visited = HashSet::new();
    let mut types = Vec::new();
    let mut budget = 512;
    let mut reaches_creation = false;
    while let Some((current, mut stable)) = pending.pop() {
        if !visited.insert((current, stable)) {
            continue;
        }
        if budget == 0 {
            return Some(LuaType::Unknown);
        }
        budget -= 1;
        let node = flow.get_flow_node(current)?;
        // Reaching this literal's creation ends the lifetime of the current table,
        // including a new table created on each loop iteration or reassignment.
        if flow.get_flow_effects(current).iter().any(|effect| {
            matches!(effect, FlowEffect::AssignDecl { value_syntax, .. }
                if value_syntax.get_range() == table.value)
        }) {
            types.push(initial.clone());
            reaches_creation = true;
            continue;
        }
        if let Some(value) = flow.get_flow_effects(current).iter().find_map(|effect| {
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect
                && receiver_decl.as_ref() == Some(decl)
                && value_syntax.get_range().end() <= offset
            {
                Some(*value_syntax)
            } else {
                None
            }
        }) {
            let replacement = model.type_of_expr(value);
            if replacement == *receiver {
                // An alias assignment preserves the table's preceding writes.
            } else if let LuaType::TableConst(_) = &replacement {
                let mut fields = vec![LuaType::Nil];
                fields.extend(
                    model
                        .member_infos(&replacement)
                        .into_iter()
                        .filter(|info| matches!(&info.key, crate::LuaMemberKey::Name(name) if string_keys_overlap(model, key_type, &LuaType::StringConst(smol_str::SmolStr::new(name.as_str()).into()))))
                        .map(|info| info.typ),
                );
                types.push(LuaType::from_vec(fields));
                continue;
            } else {
                types.push(LuaType::Unknown);
                continue;
            }
        }
        let mut overwritten = false;
        match &node.kind {
            FlowNodeKind::Assignment(ptr) => {
                let assignment = ptr.to_node(&chunk)?;
                if !assignment.get_range().contains(offset) {
                    let (vars, values) = assignment.get_var_and_expr_list();
                    for (position, var) in vars.iter().enumerate() {
                        if let LuaVarExpr::NameExpr(name) = var {
                            if key_decl.is_some()
                                && model.resolve_name(name.get_position()) == key_decl
                            {
                                stable = false;
                            }
                            continue;
                        }
                        let LuaVarExpr::IndexExpr(index) = var else {
                            continue;
                        };
                        let prefix = index.get_prefix_expr()?;
                        let target =
                            model.type_of_expr_at(prefix.get_syntax_id(), prefix.get_position());
                        if &target != receiver {
                            continue;
                        }
                        let (overlaps, same_key) = match index.get_index_key()? {
                            LuaIndexKey::Expr(expr) => {
                                let ty = model
                                    .type_of_expr_at(expr.get_syntax_id(), expr.get_position());
                                (
                                    string_keys_overlap(model, key_type, &ty),
                                    stable
                                        && key_decl.is_some()
                                        && name_decl(model, &expr) == key_decl,
                                )
                            }
                            LuaIndexKey::Name(name) => (
                                string_keys_overlap(
                                    model,
                                    key_type,
                                    &LuaType::StringConst(
                                        smol_str::SmolStr::new(name.get_name_text()).into(),
                                    ),
                                ),
                                stable && string_literal(key_type) == Some(name.get_name_text()),
                            ),
                            LuaIndexKey::String(value) => (
                                string_keys_overlap(
                                    model,
                                    key_type,
                                    &LuaType::StringConst(
                                        smol_str::SmolStr::new(value.get_value()).into(),
                                    ),
                                ),
                                stable
                                    && string_literal(key_type) == Some(value.get_value().as_str()),
                            ),
                            _ => (false, false),
                        };
                        if !overlaps {
                            continue;
                        }
                        let ty = assignment_value(model, &values, position);
                        types.push(ty);
                        if same_key {
                            overwritten = true;
                            break;
                        }
                    }
                }
            }
            // Iteration keys and calls may change before the earlier write is observed.
            FlowNodeKind::LoopLabel | FlowNodeKind::CallExprStat(_) => stable = false,
            FlowNodeKind::Unreachable => continue,
            _ => {}
        }
        if overwritten {
            continue;
        }
        match &node.antecedent {
            Some(FlowAntecedent::Single(next)) => pending.push((*next, stable)),
            Some(FlowAntecedent::Multiple(id)) => {
                pending.extend(
                    flow.get_multi_antecedents(*id)?
                        .iter()
                        .map(|next| (*next, stable)),
                );
            }
            None => types.push(initial.clone()),
        }
    }
    if reaches_creation {
        // The CFG intentionally omits some loop backedges. Writes in a surrounding
        // loop can reach a later iteration only when the table predates that loop.
        let closure = LuaClosureId::from_node(key.syntax());
        for node in key.syntax().ancestors() {
            if matches!(node.kind(), LuaKind::Syntax(LuaSyntaxKind::ClosureExpr)) {
                break;
            }
            if !matches!(
                node.kind(),
                LuaKind::Syntax(
                    LuaSyntaxKind::ForStat
                        | LuaSyntaxKind::ForRangeStat
                        | LuaSyntaxKind::WhileStat
                        | LuaSyntaxKind::RepeatStat
                )
            ) || node.text_range().contains_range(table.value)
            {
                continue;
            }
            for descendant in node.descendants() {
                if budget == 0 {
                    return Some(LuaType::Unknown);
                }
                budget -= 1;
                let Some(assignment) = LuaAssignStat::cast(descendant) else {
                    continue;
                };
                if LuaClosureId::from_node(assignment.syntax()) != closure {
                    continue;
                }
                let (vars, values) = assignment.get_var_and_expr_list();
                for (position, var) in vars.iter().enumerate() {
                    let LuaVarExpr::IndexExpr(index) = var else {
                        continue;
                    };
                    let prefix = index.get_prefix_expr()?;
                    if model.type_of_expr_at(prefix.get_syntax_id(), prefix.get_position())
                        != *receiver
                    {
                        continue;
                    }
                    let overlaps = match index.get_index_key()? {
                        LuaIndexKey::Expr(expr) => string_keys_overlap(
                            model,
                            key_type,
                            &model.type_of_expr_at(expr.get_syntax_id(), expr.get_position()),
                        ),
                        LuaIndexKey::Name(name) => string_keys_overlap(
                            model,
                            key_type,
                            &LuaType::StringConst(
                                smol_str::SmolStr::new(name.get_name_text()).into(),
                            ),
                        ),
                        LuaIndexKey::String(value) => string_keys_overlap(
                            model,
                            key_type,
                            &LuaType::StringConst(smol_str::SmolStr::new(value.get_value()).into()),
                        ),
                        _ => false,
                    };
                    if overlaps {
                        types.push(assignment_value(model, &values, position));
                    }
                }
            }
        }
    }
    Some(LuaType::from_vec(types))
}

fn name_decl(model: &SemanticModel, expr: &LuaExpr) -> Option<SemanticId> {
    match expr {
        LuaExpr::NameExpr(name) => model.resolve_name(name.get_position()),
        LuaExpr::ParenExpr(paren) => name_decl(model, &paren.get_expr()?),
        _ => None,
    }
}

fn string_literal(ty: &LuaType) -> Option<&str> {
    match ty {
        LuaType::StringConst(value) | LuaType::DocStringConst(value) => Some(value.as_str()),
        _ => None,
    }
}

fn may_be_string(model: &SemanticModel, ty: &LuaType) -> bool {
    match expand_alias_head(model, ty).as_ref().unwrap_or(ty) {
        LuaType::String
        | LuaType::StringConst(_)
        | LuaType::DocStringConst(_)
        | LuaType::Any
        | LuaType::Unknown => true,
        LuaType::Union(union) => union.iter().any(|ty| may_be_string(model, &ty)),
        _ => false,
    }
}

fn string_keys_overlap(model: &SemanticModel, left: &LuaType, right: &LuaType) -> bool {
    let left = expand_alias_head(model, left).unwrap_or_else(|| left.clone());
    let right = expand_alias_head(model, right).unwrap_or_else(|| right.clone());
    if let LuaType::Union(union) = &left {
        return union
            .iter()
            .any(|part| string_keys_overlap(model, &part, &right));
    }
    if let LuaType::Union(union) = &right {
        return union
            .iter()
            .any(|part| string_keys_overlap(model, &left, &part));
    }
    if let (Some(left), Some(right)) = (string_literal(&left), string_literal(&right)) {
        return left == right;
    }
    may_be_string(model, &left) && may_be_string(model, &right)
}

fn assignment_value(model: &SemanticModel, values: &[LuaExpr], position: usize) -> LuaType {
    let Some(last) = values.len().checked_sub(1) else {
        return LuaType::Nil;
    };
    let value = &values[position.min(last)];
    let slot = position.saturating_sub(last);
    model
        .type_of_expr_at(value.get_syntax_id(), value.get_position())
        .get_result_slot_type(slot)
        .unwrap_or(LuaType::Nil)
}
