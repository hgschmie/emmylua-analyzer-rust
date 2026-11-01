//! Conservative loop-entry transfer for declarations written by a later iteration.
//!
//! Re-entry uses the widened declaration as a cycle seed. An RHS already in
//! progress is skipped and marks the result incomplete; cycles discovered during
//! RHS inference are caught by the surrounding incompleteness-revision check.
//! Incomplete results remain provisional only for the current query. The outer
//! query scope discards them so a later query can retry with completed inputs.
//!
//! This transfer collects same-closure syntactic assignments. It does not summarize
//! captured-local mutations performed by called nested closures; that requires
//! interprocedural value effects beyond this loop-entry approximation.

#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod zero_iteration_tests;

use super::{
    FlowId, LuaType, SemanticId, SemanticModel, cache::CacheEntry, type_of_decl_at_flow_id,
    widen_const_type,
};
use emmylua_parser::{
    LuaAstNode, LuaClosureExpr, LuaExpr, LuaLiteralToken, LuaStat, LuaSyntaxId, LuaVarExpr,
};

mod numeric_loops;
mod reachability;

pub(super) fn loop_carried_type(
    model: &SemanticModel,
    decl: &SemanticId,
    syntax: LuaSyntaxId,
    entry: FlowId,
) -> Option<LuaType> {
    // Loop syntax and entry belong to this querying file view. A foreign
    // declaration has no transfer here and must not poison its owning view.
    // For this immutable CFG, entry is the first antecedent of the unique loop
    // label for syntax; it cannot vary independently of this cache key.
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
            // A provisional value is usable as a seed, never as a completed fact.
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
    // A nested incomplete dependency keeps even a None result provisional; the
    // query must retry rather than publish that temporary absence as a ready fact.
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
    // Foreign declarations have no transfer in this file view.
    let info = facts.decl_by_id(decl)?;
    let tree = model.syntax_tree()?;
    let node = syntax.to_node_from_root(&tree.get_red_root())?;
    // A local created inside the loop starts fresh each iteration.
    if matches!(info.kind, crate::semantic_db::def::DeclKind::Local { .. })
        && node.text_range().contains_range(info.name_range)
    {
        return None;
    }
    let closure = node
        .ancestors()
        .find_map(LuaClosureExpr::cast)
        .map(|c| c.get_syntax_id());
    // Include nested loops in this closure: their writes can reach a later outer
    // iteration. Repeated types collapse during widening. Nested closure bodies
    // are excluded; calling them needs a separate captured-value effect summary.
    let flow_tree = model.flow_tree()?;
    let reachable = reachability::reachable_flows(model);
    let mut assigned = Vec::new();
    // The first query scans this loop body; completed transfers are cached per
    // declaration/loop, and reachability is shared across declarations in the file.
    for statement in node.descendants().filter_map(LuaStat::cast) {
        if statement
            .ancestors::<LuaClosureExpr>()
            .next()
            .map(|c| c.get_syntax_id())
            != closure
        {
            continue;
        }
        if flow_tree
            .get_flow_id_at(statement.get_position())
            .and_then(|flow| reachable.get(flow.0 as usize))
            == Some(&false)
        {
            continue;
        }
        // Numeric loops with a statically empty range retain a possible body in
        // the CFG. Its writes cannot contribute to this outer loop transfer.
        if numeric_loops::is_in_unentered_body(model, &statement) {
            continue;
        }
        let assignment = match statement {
            LuaStat::AssignStat(assignment) => assignment,
            LuaStat::FuncStat(function) => {
                // A function statement installs its closure in the named target;
                // its body is not executed. Qualified members do not rebind a local.
                if let Some(LuaVarExpr::NameExpr(name)) = function.get_func_name()
                    && model.resolve_name(name.get_position()).as_ref() == Some(decl)
                    && let Some(body) = function.get_closure()
                {
                    assigned.push(Some((body.get_syntax_id(), 0)));
                }
                continue;
            }
            _ => continue,
        };
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
                        // final call or bare vararg can supply additional result slots.
                        // For a, b, c = x, f(), c uses result slot 1 of f().
                        values
                            .last()
                            .filter(|value| expands_result_slots(value))
                            .map(|value| (value.get_syntax_id(), index + 1 - values.len()))
                    });
                assigned.push(value);
            }
        }
    }
    if assigned.is_empty() {
        return None;
    }
    // Declared contracts bound all iterations; assignment checking enforces them.
    // Parameter docs are keyed by name, not their order in the comment. A local
    // shadow with the same name must not inherit the parameter's contract.
    let annotated_param = matches!(info.kind, crate::semantic_db::def::DeclKind::Param)
        && facts
            .signature_and_param_index_of_decl(info)
            .and_then(|(signature, _)| signature.docs.as_ref())
            .is_some_and(|docs| docs.param_types.iter().any(|(name, _)| name == &info.name));
    if info.doc_type_syntax.is_some() || annotated_param {
        return model.type_of_decl(decl);
    }
    // This transfer widens syntactic writes; it is not a general iterative
    // solver for recursively evolving structural types. Unresolved RHS queries
    // retain uncertainty and keep their result provisional for later queries.
    // A recursively initialized global can have an incomplete declaration.
    // Keep its unknown entry seed while still collecting independent later writes.
    let initial = type_of_decl_at_flow_id(model, decl, entry);
    let structural_entry = has_structural_shape(&initial);
    let mut types = vec![widen_const_type(initial)];
    for rhs in assigned {
        let self_dependent = rhs.is_some_and(|(syntax, _)| {
            references_decl(model, syntax, decl) && !is_identity_write(model, syntax, decl)
        });
        // A single transfer cannot enumerate recursively shrinking/growing
        // structures (for example value = value.next). Keep explicit uncertainty
        // instead of presenting the first two shapes as the entire loop domain.
        if self_dependent && structural_entry {
            types.push(LuaType::Unknown);
        }
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
            // Leave a provisional result so a later query retries after the outer
            // expression completes; this does not schedule another query.
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
                    // Result slots unpack expression values, not callable return
                    // types: a closure is itself the value at slot zero.
                    ty.get_result_slot_type(slot).unwrap_or(LuaType::Nil)
                }
            })
            .unwrap_or(LuaType::Nil);
        if self_dependent && has_structural_shape(&typ) {
            types.push(LuaType::Unknown);
        }
        types.push(widen_const_type(typ));
    }
    Some(LuaType::from_vec(types))
}

fn has_structural_shape(typ: &LuaType) -> bool {
    match typ {
        LuaType::TableConst(_)
        | LuaType::Object(_)
        | LuaType::Array(_)
        | LuaType::Tuple(_)
        | LuaType::Instance(_)
        | LuaType::Generic(_)
        | LuaType::TableGeneric(_)
        | LuaType::Intersection(_)
        | LuaType::Conditional(_)
        | LuaType::Mapped(_) => true,
        LuaType::Union(union) => union.into_vec().iter().any(has_structural_shape),
        _ => false,
    }
}

/// Assigning the same receiver unchanged cannot evolve its structural domain.
fn is_identity_write(model: &SemanticModel, syntax: LuaSyntaxId, decl: &SemanticId) -> bool {
    model.chunk().and_then(|chunk| syntax.to_node_from_root(chunk.syntax()))
        .and_then(LuaExpr::cast).and_then(super::guards::unparenthesized)
        .is_some_and(|expr| matches!(expr, LuaExpr::NameExpr(name) if model.resolve_name(name.get_position()).as_ref() == Some(decl)))
}

/// Parentheses suppress Lua's final-expression expansion. Dot and colon method
/// calls use the same CallExpr representation as other calls.
fn expands_result_slots(value: &LuaExpr) -> bool {
    match value {
        LuaExpr::CallExpr(_) => true,
        LuaExpr::LiteralExpr(literal) => {
            matches!(literal.get_literal(), Some(LuaLiteralToken::Dots(_)))
        }
        _ => false,
    }
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
    // Name identity comes from immutable file facts, not provisional types.
    // resolve_name does not infer types or re-enter this syntax-only query, so
    // this cache needs no type-query InProgress sentinel.
    let references = model
        .syntax_tree()
        .and_then(|tree| syntax.to_node_from_root(&tree.get_red_root()))
        .is_some_and(|node| reads_decl(model, node, decl));
    model
        .cache
        .borrow_mut()
        .loop_rhs_dependencies
        .insert(key, references);
    references
}

/// Creating a closure does not evaluate its body. A closure passed to or used as
/// a call may execute during the RHS, so retain that dependency conservatively.
fn reads_decl(
    model: &SemanticModel,
    root: emmylua_parser::LuaSyntaxNode,
    decl: &SemanticId,
) -> bool {
    let range = root.text_range();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if LuaClosureExpr::can_cast(node.kind().into())
            && !node
                .ancestors()
                .skip(1)
                .take_while(|ancestor| {
                    range.contains_range(ancestor.text_range())
                        && !LuaClosureExpr::can_cast(ancestor.kind().into())
                })
                .any(|ancestor| emmylua_parser::LuaCallExpr::can_cast(ancestor.kind().into()))
        {
            continue;
        }
        if let Some(name) = emmylua_parser::LuaNameExpr::cast(node.clone())
            && model.resolve_name(name.get_position()).as_ref() == Some(decl)
        {
            return true;
        }
        pending.extend(node.children());
    }
    false
}
