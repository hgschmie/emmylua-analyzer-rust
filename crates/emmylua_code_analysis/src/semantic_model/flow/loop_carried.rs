//! Conservative loop-entry transfer for declarations written by a later iteration.
use super::{LuaType, SemanticId, SemanticModel, cache::CacheEntry, widen_const_type};
use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaClosureExpr, LuaSyntaxId, LuaVarExpr};

pub(super) fn loop_carried_type(
    model: &SemanticModel,
    decl: &SemanticId,
    syntax: LuaSyntaxId,
    entry: super::FlowId,
) -> Option<LuaType> {
    let key = (syntax, decl.clone());
    let cached = model.cache.borrow().loop_carried.get(&key).cloned();
    if let Some(cached) = cached {
        return match cached {
            CacheEntry::Ready(ty) => ty,
            CacheEntry::InProgress => model.type_of_decl(decl).map(widen_const_type),
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
    entry: super::FlowId,
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
    let annotated_param = facts
        .signature_and_param_index_of_decl(info)
        .and_then(|(signature, _)| signature.docs.as_ref())
        .is_some_and(|docs| docs.param_types.iter().any(|(name, _)| name == &info.name));
    if info.doc_type_syntax.is_some() || annotated_param {
        return Some(declared);
    }
    let initial = super::type_of_decl_at_flow_id(model, decl, entry);
    let mut types = vec![widen_const_type(initial)];
    for rhs in assigned {
        let typ = rhs
            .map(|(syntax, slot)| {
                let ty = model.type_of_expr(syntax);
                if matches!(ty, LuaType::Unknown | LuaType::Any) {
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
    model
        .syntax_tree()
        .and_then(|tree| syntax.to_node_from_root(&tree.get_red_root()))
        .is_some_and(|node| {
            node.descendants()
                .filter_map(emmylua_parser::LuaNameExpr::cast)
                .any(|name| model.resolve_name(name.get_position()).as_ref() == Some(decl))
        })
}
