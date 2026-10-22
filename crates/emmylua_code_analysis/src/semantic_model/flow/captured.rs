//! Entry values for locals shared across closure invocations.

use super::{FlowEffect, FlowTree, LuaClosureId, LuaType, SemanticId, SemanticModel};
use crate::DeclKind;
use crate::semantic_model::cache::CacheEntry;

pub(super) fn entry_type(
    model: &SemanticModel,
    decl: &SemanticId,
    tree: &FlowTree,
    closure: LuaClosureId,
) -> Option<LuaType> {
    let facts = model.file_facts()?;
    let info = facts.decl_by_id(decl)?;
    if closure.get_range().contains_range(info.name_range)
        || !matches!(info.kind, DeclKind::Local { .. } | DeclKind::Param)
        || !tree.has_decl_assignment(decl)
    {
        return None;
    }
    let declared = model.type_of_decl(decl).unwrap_or(LuaType::Unknown);
    let annotated_param = facts
        .signature_and_param_index_of_decl(info)
        .and_then(|(signature, _)| signature.docs.as_ref())
        .is_some_and(|docs| docs.param_types.iter().any(|(name, _)| name == &info.name));
    if info.doc_type_syntax.is_some() || annotated_param {
        return Some(declared);
    }
    let initial = if declared.is_unknown()
        && info.value_expr_syntax.is_none()
        && matches!(info.kind, DeclKind::Local { is_iter: false, .. })
    {
        LuaType::Nil
    } else {
        declared
    };
    let key = (model.file_id(), decl.clone());
    let cached = model.cache.borrow().captured_decl.get(&key).cloned();
    if let Some(cached) = cached {
        let cache = model.cache.borrow();
        // Recursive assignments use the declared value as a seed; incomplete
        // summaries are retried at the next outer query boundary.
        return Some(
            cached
                .get(&cache)
                .cloned()
                .unwrap_or_else(|| widen(initial)),
        );
    }
    model
        .cache
        .borrow_mut()
        .captured_decl
        .insert(key.clone(), CacheEntry::InProgress);
    let revision = model.cache.borrow().revision();
    let mut types = vec![initial.clone()];
    for &(position, flow) in tree.decl_assignments(decl) {
        for effect in tree.get_flow_effects(flow) {
            if let FlowEffect::AssignDecl {
                decl: target,
                value_syntax,
            } = effect
                && target == decl
            {
                types.push(super::assigned_value_type(
                    model,
                    tree,
                    decl,
                    flow,
                    position,
                    *value_syntax,
                    &initial,
                ));
            }
        }
    }
    let result = LuaType::from_vec(types);
    let mut cache = model.cache.borrow_mut();
    let entry = CacheEntry::finish(result.clone(), cache.is_complete(revision));
    cache.captured_decl.insert(key, entry);
    Some(result)
}

fn widen(ty: LuaType) -> LuaType {
    match ty {
        LuaType::BooleanConst(_) | LuaType::DocBooleanConst(_) => LuaType::Boolean,
        LuaType::Union(union) => LuaType::from_vec(union.iter().cloned().map(widen).collect()),
        other => super::widen_const_type(other),
    }
}
