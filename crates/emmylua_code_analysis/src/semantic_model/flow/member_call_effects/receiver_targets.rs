//! Discover and cache fixed receiver paths; actual invocation bindings stay local.

#[cfg(test)]
use super::RECEIVER_TARGET_SCANS;
use super::{Access, SemanticModel, guards, literal_member_key, member_access_path};
use crate::{LuaMemberKey, SemanticId};
use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaExpr, LuaIndexExpr};
use std::{collections::HashSet, rc::Rc};

pub(super) fn receiver_targets(model: &SemanticModel, expected: &Access) -> Rc<Vec<Access>> {
    let mut targets = fixed_receiver_targets(model);
    if !targets.contains(expected) {
        // Fixed exact paths are already shared. Unresolved/dynamic targets may
        // need a query-local extension; retaining each full extension would
        // multiply cache memory by the number of uncertain access paths.
        Rc::make_mut(&mut targets).push(expected.clone());
    }
    targets
}

fn fixed_receiver_targets(model: &SemanticModel) -> Rc<Vec<Access>> {
    if let Some(targets) = model
        .cache
        .borrow()
        .member_receiver_targets
        .get(&model.file_id())
    {
        return targets.clone();
    }
    let _query = crate::semantic_model::cache::QueryCacheScope::new(&model.cache);
    let revision = model.cache.borrow().revision();
    #[cfg(test)]
    RECEIVER_TARGET_SCANS.with(|count| count.set(count.get() + 1));
    let mut targets = Vec::new();
    // Preserve deterministic discovery order without repeated structural scans
    // of every previously discovered receiver path.
    let mut seen = HashSet::new();
    if let Some(chunk) = model.chunk() {
        // Syntax fixes the finite set of roots and path lengths. Include callable
        // paths separately from the guarded-write target, so helper(methods) can
        // retain a method table unrelated to the guarded receiver.
        for index in chunk.descendants::<LuaIndexExpr>() {
            let expr = LuaExpr::IndexExpr(index);
            if let Some(path) = member_access_path(model, &expr)
                && !path.1.iter().any(LuaMemberKey::is_none)
                && seen.insert(path.clone())
            {
                targets.push(path);
            }
        }
    }
    if let Some(chunk) = model.chunk() {
        let mut tables = Vec::new();
        for local in chunk.descendants::<emmylua_parser::LuaLocalStat>() {
            for (name, value) in local.get_local_name_list().zip(local.get_value_exprs()) {
                tables.push((
                    (
                        SemanticId::decl(model.file_id(), name.get_range()),
                        Vec::new(),
                    ),
                    value,
                ));
            }
        }
        for assign in chunk.descendants::<LuaAssignStat>() {
            let (vars, values) = assign.get_var_and_expr_list();
            for (var, value) in vars.into_iter().zip(values) {
                if let Some(path) = LuaExpr::cast(var.syntax().clone())
                    .and_then(|expr| member_access_path(model, &expr))
                {
                    tables.push((path, value));
                }
            }
        }
        while let Some((path, value)) = tables.pop() {
            let Some(LuaExpr::TableExpr(table)) = guards::unparenthesized(value) else {
                continue;
            };
            for field in table.get_fields() {
                let Some(key) = field
                    .get_field_key()
                    .map(literal_member_key)
                    .filter(|key| !key.is_none())
                else {
                    continue;
                };
                let mut child = path.clone();
                child.1.push(key);
                if seen.insert(child.clone()) {
                    targets.push(child.clone());
                }
                if let Some(value) = field.get_value_expr() {
                    tables.push((child, value));
                }
            }
        }
    }
    let targets = Rc::new(targets);
    model.cache.publish_if_complete(revision, |cache| {
        cache
            .member_receiver_targets
            .insert(model.file_id(), targets.clone());
    });
    targets
}
