//! Conservative loop-entry transfer for declarations written by a later iteration.
//!
//! Re-entry uses the widened declaration as a cycle seed. An RHS already in
//! progress is skipped and marks the result incomplete; cycles discovered during
//! RHS inference are caught by the surrounding incompleteness-revision check.
//! Incomplete results remain provisional only for the current query. The outer
//! query scope discards them so a later query can retry with completed inputs.

#[cfg(test)]
mod tests;

use super::{
    FlowId, LuaType, SemanticId, SemanticModel, cache::CacheEntry, type_of_decl_at_flow_id,
    widen_const_type,
};
use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaClosureExpr, LuaSyntaxId, LuaVarExpr};

pub(super) fn loop_carried_type(
    model: &SemanticModel,
    decl: &SemanticId,
    syntax: LuaSyntaxId,
    entry: FlowId,
) -> Option<LuaType> {
    // Loop syntax and entry belong to this querying file view. A foreign
    // declaration has no transfer here and must not poison its owning view.
    let key = (model.file_id(), syntax, decl.clone());
    let cached = model.cache.borrow().loop_carried.get(&key).cloned();
    if let Some(cached) = cached {
        return match cached {
            CacheEntry::Ready(ty) => ty,
            // A cycle has no completed loop result yet; seed from the declaration.
            CacheEntry::InProgress => {
                model.cache.borrow().mark_incomplete();
                model.type_of_decl(decl).map(widen_const_type)
            }
            CacheEntry::Provisional(ty) => {
                model.cache.borrow().mark_incomplete();
                ty
            }
        };
    }
    model
        .cache
        .borrow_mut()
        .loop_carried
        .insert(key.clone(), CacheEntry::InProgress);
    let revision = model.cache.borrow().revision();
    let value = compute(model, decl, syntax, entry);
    let mut cache = model.cache.borrow_mut();
    let result = CacheEntry::finish(value.clone(), cache.is_complete(revision));
    cache.loop_carried.insert(key, result);
    value
}

fn compute(
    model: &SemanticModel,
    decl: &SemanticId,
    syntax: LuaSyntaxId,
    entry: FlowId,
) -> Option<LuaType> {
    let facts = model.file_facts()?;
    let info = facts.decl_by_id(decl)?;
    let tree = model.syntax_tree()?;
    let node = syntax.to_node_from_root(&tree.get_red_root())?;
    // A local created inside the loop starts fresh each iteration.
    if node.text_range().contains_range(info.name_range) {
        return None;
    }
    let closure = node
        .ancestors()
        .find_map(LuaClosureExpr::cast)
        .map(|c| c.get_syntax_id());
    // Include nested loops in this closure: their writes can reach a later outer
    // iteration. Repeated types collapse during widening; uncalled closures do not.
    let mut assigned = Vec::new();
    for assignment in node.descendants().filter_map(LuaAssignStat::cast) {
        if assignment
            .ancestors::<LuaClosureExpr>()
            .next()
            .map(|c| c.get_syntax_id())
            != closure
        {
            continue;
        }
        let (vars, values) = assignment.get_var_and_expr_list();
        for (index, var) in vars.iter().enumerate() {
            if let LuaVarExpr::NameExpr(name) = var
                && model.resolve_name(name.get_position()).as_ref() == Some(decl)
            {
                let value = values
                    .get(index)
                    .map(|value| (value.get_syntax_id(), 0))
                    .or_else(|| {
                        // get(index) failed, so index >= values.len(). Only a
                        // final call can supply additional nonzero result slots.
                        values
                            .last()
                            .filter(|value| matches!(value, emmylua_parser::LuaExpr::CallExpr(_)))
                            .map(|value| (value.get_syntax_id(), index + 1 - values.len()))
                    });
                assigned.push(value);
            }
        }
    }
    if assigned.is_empty() {
        return None;
    }
    let declared = model.type_of_decl(decl)?;
    // Declared contracts bound all iterations; assignment checking enforces them.
    // Parameter docs are keyed by name, not their order in the comment. A local
    // shadow with the same name must not inherit the parameter's contract.
    let annotated_param = matches!(info.kind, crate::semantic_db::def::DeclKind::Param)
        && facts
            .signature_and_param_index_of_decl(info)
            .and_then(|(signature, _)| signature.docs.as_ref())
            .is_some_and(|docs| docs.param_types.iter().any(|(name, _)| name == &info.name));
    if info.doc_type_syntax.is_some() || annotated_param {
        return Some(declared);
    }
    let initial = type_of_decl_at_flow_id(model, decl, entry);
    let mut types = vec![widen_const_type(initial)];
    for rhs in assigned {
        if let Some((syntax, _)) = rhs
            && matches!(
                model
                    .cache
                    .borrow()
                    .expr_type
                    .get(&(model.file_id(), syntax)),
                Some(CacheEntry::InProgress)
            )
        {
            // The current RHS asks for its own loop-entry input. Seed that
            // cycle from the entry and other writes, not an unknown RHS value.
            // Recompute after the outer expression completes.
            model.cache.borrow().mark_incomplete();
            continue;
        }
        let typ = rhs
            .map(|(syntax, slot)| {
                let ty = model.type_of_expr(syntax);
                if matches!(ty, LuaType::Unknown | LuaType::Any) {
                    // An unresolved whole call gives no reliable return slots;
                    // preserve uncertainty instead of claiming a later slot is nil.
                    ty
                } else {
                    ty.get_result_slot_type(slot).unwrap_or(LuaType::Nil)
                }
            })
            .unwrap_or(LuaType::Nil);
        types.push(widen_const_type(typ));
    }
    Some(LuaType::from_vec(types))
}

pub(super) fn references_decl(
    model: &SemanticModel,
    syntax: LuaSyntaxId,
    decl: &SemanticId,
) -> bool {
    let key = (model.file_id(), syntax, decl.clone());
    if let Some(references) = model.cache.borrow().loop_rhs_dependencies.get(&key) {
        return *references;
    }
    // Name identity comes from the immutable file facts, not provisional types.
    let references = model
        .syntax_tree()
        .and_then(|tree| syntax.to_node_from_root(&tree.get_red_root()))
        .is_some_and(|node| {
            node.descendants()
                .filter_map(emmylua_parser::LuaNameExpr::cast)
                .any(|name| model.resolve_name(name.get_position()).as_ref() == Some(decl))
        });
    model
        .cache
        .borrow_mut()
        .loop_rhs_dependencies
        .insert(key, references);
    references
}
