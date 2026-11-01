//! Visible call effects for member guards. Opaque calls preserve existing guards.
//! This traversal detects visible writes; it never propagates callee values.
//! Passing a receiver to an opaque call alone does not invalidate guards. Writes
//! behind unresolved alias/receiver resolution may therefore go unreported.
//! Same-file fixed member paths use reaching assignments and actual dot/colon
//! receivers. Dynamic callees and interprocedural installation of methods remain
//! opaque. Factory-returned closures are also opaque: completed calls do not
//! provide captured closure-creation environments. This includes installing a callable with rawset: its direct write
//! invalidates the member, but the stored callable is not a value postcondition.
//! Captured member replacements without an invocation context are opaque.
//! A reassigned name callee captured in a different closure is also
//! opaque; the presence of any declaration assignment triggers that boundary.
//!
//! Syntax-only caches live for the immutable model snapshot. Type-dependent
//! receiver targets use publish_if_complete so provisional inference is retried;
//! invocation histories share immutable binding snapshots and retain full equality.

use std::{
    borrow::Cow,
    collections::{HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    rc::Rc,
};

use emmylua_parser::{
    LuaAstNode, LuaCallExpr, LuaClosureExpr, LuaExpr, LuaFuncStat, LuaIndexExpr, LuaSyntaxId,
    LuaSyntaxNode, LuaVarExpr,
};
use rowan::TextSize;

use super::{SemanticModel, guards, index_access_path, member_access_path, path_write_affects};
use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::LuaClosureId;
use crate::{LuaMemberKey, LuaType};

mod global_values;
mod loop_members;
mod member_values;
mod path_order;
mod reaching_values;
mod receiver_targets;
mod scan;
pub(super) use global_values::global_name_values;
pub(super) use loop_members::{
    crosses_boundary as crosses_loop_write_boundary, write_boundaries as loop_write_boundaries,
};
use member_values::{literal_member_key, member_values};
use reaching_values::{callee_reachable_flows, reaching_values};
use receiver_targets::receiver_targets;
use scan::scan;

type Access = (SemanticId, Vec<LuaMemberKey>);
type Bindings = Vec<(SemanticId, Vec<Access>)>;
// Histories contain each (callee, callsite, caller bindings) state at most once.
// This bounds recursion without merging replacement order or caller receivers.
#[derive(Clone, PartialEq, Eq)]
struct Invocation {
    closure: LuaClosureId,
    site: LuaSyntaxId,
    bindings: Rc<Bindings>,
    // Construct only through new(): the fingerprint is a deterministic function
    // of the immutable bindings snapshot and must never be edited independently.
    // Equality still compares bindings, so a fingerprint collision is harmless.
    bindings_hash: u64,
}

impl Invocation {
    fn new(closure: LuaClosureId, site: LuaSyntaxId, bindings: &Bindings) -> Self {
        let mut state = DefaultHasher::new();
        bindings.hash(&mut state);
        Self {
            closure,
            site,
            bindings: Rc::new(bindings.clone()),
            bindings_hash: state.finish(),
        }
    }
}

impl Hash for Invocation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.closure.hash(state);
        self.site.hash(state);
        // Equality still compares complete semantic bindings on hash collisions.
        // Sharing immutable snapshots avoids copying every ancestor's formals.
        self.bindings_hash.hash(state);
    }
}

type Invocations = Vec<Invocation>;
type CalleeValue = (LuaExpr, Bindings, Invocations);

fn relevant(path: &Access, targets: &[Access]) -> bool {
    targets
        .iter()
        .any(|target| path_write_affects(path, &target.0, &target.1))
}

#[cfg(test)]
thread_local! { static RECEIVER_TARGET_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Cache whether calls or member function definitions require path-aware flow.
/// Earlier calls can invalidate a read; function statements can replace an
/// existing member even when the file contains no calls or conditional branches.
pub(super) fn file_requires_member_flow(model: &SemanticModel) -> bool {
    if let Some(present) = model
        .cache
        .borrow()
        .member_file_requires_flow
        .get(&model.file_id())
    {
        return *present;
    }
    let present = model.chunk().is_some_and(|chunk| {
        chunk.descendants::<LuaCallExpr>().next().is_some()
            || chunk
                .descendants::<LuaFuncStat>()
                .any(|function| matches!(function.get_func_name(), Some(LuaVarExpr::IndexExpr(_))))
    });
    model
        .cache
        .borrow_mut()
        .member_file_requires_flow
        .insert(model.file_id(), present);
    present
}

pub(super) fn calls_conflict(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    node: &LuaSyntaxNode,
    before: TextSize,
) -> bool {
    calls_conflict_after(model, index, node, TextSize::from(0), before)
}

pub(super) fn calls_conflict_after(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    node: &LuaSyntaxNode,
    after: TextSize,
    before: TextSize,
) -> bool {
    let expr = LuaExpr::IndexExpr(index.clone());
    let Some(expected) = member_access_path(model, &expr) else {
        return false;
    };
    scan(model, &expected, node, after, before)
}

fn access_paths(
    model: &SemanticModel,
    expr: &LuaExpr,
    bindings: &Bindings,
    targets: &[Access],
) -> Vec<Access> {
    let mut pending = vec![(expr.clone(), Vec::new())];
    let mut seen = HashSet::new();
    let mut paths = HashSet::new();
    while let Some((expr, suffix)) = pending.pop() {
        if !seen.insert((expr.get_syntax_id(), suffix.clone())) {
            continue;
        }
        let Some((root, mut keys)) = member_access_path(model, &expr) else {
            continue;
        };
        keys.extend(suffix);
        // Alias expansion can only prepend keys. A longer path cannot affect
        // this member, and pruning it makes recursive descendant aliases finite.
        // Descendant writes cannot replace their ancestor; each reassignment is
        // resolved separately before this expansion, so pruning cannot shorten it.
        if targets.iter().all(|target| keys.len() > target.1.len()) {
            continue;
        }
        let initial = bindings.iter().find(|(param, _)| *param == root);
        let alias = if initial.is_none() {
            formal_alias_value(model, &root, bindings)
        } else {
            None
        };
        if initial.is_none() && alias.is_none() {
            let path = (root, keys);
            if relevant(&path, targets) {
                paths.insert(path);
            }
            continue;
        }
        for value in reaching_values(model, &root, &expr) {
            if let Some(value) = value.or_else(|| alias.clone()) {
                pending.push((value, keys.clone()));
            } else if let Some((_, initial)) = initial {
                for (root, prefix) in initial {
                    let mut path = (root.clone(), prefix.clone());
                    path.1.extend(keys.clone());
                    if relevant(&path, targets) {
                        paths.insert(path);
                    }
                }
            }
        }
    }
    paths.into_iter().collect()
}

/// Find a name-only initializer chain to a bound formal without reviving detached member paths.
fn formal_alias_value(
    model: &SemanticModel,
    root: &SemanticId,
    bindings: &Bindings,
) -> Option<LuaExpr> {
    let facts = model.file_facts()?;
    let chunk = model.chunk()?;
    let mut root = root.clone();
    let mut seen = HashSet::new();
    let mut first = None;
    while seen.insert(root.clone()) {
        let decl = facts.decl_by_id(&root)?;
        if !matches!(decl.kind, crate::DeclKind::Local { is_iter: false, .. })
            || decl.multi_return_index.is_some_and(|index| index > 0)
        {
            return None;
        }
        let value = LuaExpr::cast(decl.value_expr_syntax?.to_node_from_root(chunk.syntax())?)?;
        let value = guards::unparenthesized(value)?;
        let LuaExpr::NameExpr(name) = &value else {
            return None;
        };
        root = model.resolve_name(name.get_position())?;
        first.get_or_insert(value);
        if bindings.iter().any(|(param, _)| *param == root) {
            return first;
        }
    }
    None
}

fn write_conflicts(
    model: &SemanticModel,
    expected: &Access,
    var: &LuaVarExpr,
    bindings: &Bindings,
) -> bool {
    let written = match var {
        LuaVarExpr::NameExpr(name) => {
            let Some(root) = model.resolve_name(name.get_position()) else {
                return false;
            };
            // Rebinding a callee parameter or a local alias does not replace the caller's table.
            if bindings.iter().any(|(param, _)| *param == root) {
                return false;
            }
            vec![(root, Vec::new())]
        }
        LuaVarExpr::IndexExpr(index) => access_paths(
            model,
            &LuaExpr::IndexExpr(index.clone()),
            bindings,
            std::slice::from_ref(expected),
        ),
    };
    written
        .iter()
        .any(|written| path_write_affects(written, &expected.0, &expected.1))
}

fn local_callees(
    model: &SemanticModel,
    call: &LuaCallExpr,
    bindings: &Bindings,
    targets: &[Access],
    invocations: &Invocations,
) -> Vec<LuaClosureExpr> {
    let Some(prefix) = call.get_prefix_expr() else {
        return Vec::new();
    };
    let mut callees = resolve_callees(model, prefix, bindings, targets, invocations);
    let mut seen = HashSet::new();
    callees.retain(|callee| seen.insert(callee.get_syntax_id()));
    callees
}

/// Follow local name values without invoking member lookup. Mutable captures
/// outside their defining closure remain opaque, as in the visible-call resolver.
fn local_name_values(model: &SemanticModel, expr: &LuaExpr) -> Vec<LuaExpr> {
    let LuaExpr::NameExpr(name) = expr else {
        return Vec::new();
    };
    let Some(SemanticId::Decl(key)) = model.resolve_name(name.get_position()) else {
        return Vec::new();
    };
    if model.is_std_file(key.file_id) {
        return global_name_values(model, name).replacements;
    }
    if key.file_id != model.file_id() {
        return Vec::new();
    }
    let id = SemanticId::Decl(key);
    let (Some(facts), Some(tree), Some(chunk)) =
        (model.file_facts(), model.flow_tree(), model.chunk())
    else {
        return Vec::new();
    };
    let Some(decl) = facts.decl_by_id(&id) else {
        return Vec::new();
    };
    if tree.has_decl_assignment(&id) {
        let owner = decl
            .owner_syntax
            .and_then(|owner| owner.to_node_from_root(chunk.syntax()));
        if owner.is_none_or(|owner| {
            LuaClosureId::from_node(&owner) != LuaClosureId::from_node(name.syntax())
        }) {
            return Vec::new();
        }
    }
    reaching_values(model, &id, expr)
        .into_iter()
        .filter_map(|value| {
            value.or_else(|| {
                decl.value_expr_syntax
                    .and_then(|value| value.to_node_from_root(chunk.syntax()))
                    .and_then(LuaExpr::cast)
            })
        })
        .collect()
}

/// A possible standard rawset value suffices to expose a conflicting write.
/// This name-only walk is also used during member resolution and must not call
/// that resolver recursively. Definite harmless replacements stop old aliases.
fn may_call_rawset(model: &SemanticModel, expr: LuaExpr) -> bool {
    let mut pending = vec![expr];
    let mut seen = HashSet::new();
    while let Some(expr) = pending.pop() {
        let Some(expr) = guards::unparenthesized(expr) else {
            continue;
        };
        if !seen.insert(expr.get_syntax_id()) {
            continue;
        }
        let LuaExpr::NameExpr(name) = &expr else {
            continue;
        };
        if name.get_name_text().as_deref() == Some("rawset")
            && let Some(SemanticId::Decl(decl)) = model.resolve_name(name.get_position())
            && model.is_std_file(decl.file_id)
            && global_name_values(model, name).may_keep_builtin
        {
            return true;
        }
        pending.extend(local_name_values(model, &expr));
    }
    false
}

fn resolve_callees(
    model: &SemanticModel,
    expr: LuaExpr,
    bindings: &Bindings,
    targets: &[Access],
    invocations: &Invocations,
) -> Vec<LuaClosureExpr> {
    // Context identities avoid cloning/hashing every formal for each name in
    // an alias chain. Member RHS values intern their restored caller context.
    let mut contexts = vec![(Cow::Borrowed(bindings), Cow::Borrowed(invocations))];
    let mut pending = vec![(expr, 0)];
    let mut seen = HashSet::new();
    let mut callees = Vec::new();
    while let Some((expr, context)) = pending.pop() {
        let Some(expr) = guards::unparenthesized(expr) else {
            continue;
        };
        if !seen.insert((expr.get_syntax_id(), context)) {
            continue;
        }
        match &expr {
            LuaExpr::ClosureExpr(closure) => callees.push(closure.clone()),
            LuaExpr::NameExpr(_) => {
                pending.extend(
                    local_name_values(model, &expr)
                        .into_iter()
                        .map(|value| (value, context)),
                );
            }
            LuaExpr::IndexExpr(_) => {
                let (bindings, invocations) = &contexts[context];
                let mut values = Vec::new();
                for path in access_paths(model, &expr, bindings, targets) {
                    if !path.1.iter().any(LuaMemberKey::is_none) {
                        values.extend(member_values(
                            model,
                            &path,
                            &expr,
                            bindings,
                            targets,
                            invocations,
                        ));
                    }
                }
                for (value, bindings, invocations) in values {
                    let context = contexts
                        .iter()
                        .position(|(existing_bindings, existing_invocations)| {
                            existing_bindings.as_ref() == &bindings
                                && existing_invocations.as_ref() == &invocations
                        })
                        .unwrap_or_else(|| {
                            let id = contexts.len();
                            contexts.push((Cow::Owned(bindings), Cow::Owned(invocations)));
                            id
                        });
                    pending.push((value, context));
                }
            }
            _ => {}
        }
    }
    callees
}

fn mutator_conflicts(
    model: &SemanticModel,
    expected: &Access,
    call: &LuaCallExpr,
    bindings: &Bindings,
) -> bool {
    let Some(prefix) = call.get_prefix_expr().and_then(guards::unparenthesized) else {
        return false;
    };
    if !may_call_rawset(model, prefix) {
        return false;
    }
    let Some(args) = call.get_args_list() else {
        return false;
    };
    let args = args.get_args().collect::<Vec<_>>();
    let Some(receiver) = args.first() else {
        return false;
    };
    let mut written = access_paths(model, receiver, bindings, std::slice::from_ref(expected));
    let key_type = args
        .get(1)
        .map(|key| model.type_of_expr(key.get_syntax_id()));
    let numeric = key_type.as_ref().is_some_and(LuaType::is_number);
    let non_integer = matches!(&key_type, Some(LuaType::FloatConst(value)) if crate::semantic_db::types::exact_integer(*value).is_none());
    let key = match key_type {
        Some(LuaType::StringConst(key) | LuaType::DocStringConst(key)) => {
            LuaMemberKey::Name((*key).clone())
        }
        Some(LuaType::IntegerConst(key) | LuaType::DocIntegerConst(key)) => {
            LuaMemberKey::Integer(key)
        }
        Some(LuaType::FloatConst(value)) => crate::semantic_db::types::exact_integer(value)
            .map(LuaMemberKey::Integer)
            .unwrap_or(LuaMemberKey::None),
        _ => LuaMemberKey::None,
    };
    written.iter_mut().any(|path| {
        // A raw write affects this next segment, including when it replaces an
        // ancestor of the guarded member. Unknown key domains stay conservative.
        let segment = expected.1.get(path.1.len());
        if numeric && segment.is_some_and(LuaMemberKey::is_name)
            || non_integer && segment.is_some_and(LuaMemberKey::is_integer)
        {
            return false;
        }
        path.1.push(key.clone());
        path_write_affects(path, &expected.0, &expected.1)
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod rawset_key_tests;

#[cfg(test)]
mod loop_callee_tests;

#[cfg(test)]
mod query_contract_tests;

#[cfg(test)]
mod captured_fixed_review_tests;
