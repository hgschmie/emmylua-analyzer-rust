//! Visible call effects for member guards. Opaque calls preserve existing guards.
//! This traversal detects visible writes; it never propagates callee values.
//! Passing a receiver to an opaque call alone does not invalidate guards. Writes
//! behind unresolved alias/receiver resolution may therefore go unreported.
//! Same-file fixed member paths use reaching assignments and actual dot/colon
//! receivers. Dynamic callees and interprocedural installation of methods remain
//! opaque. Captured member replacements without an invocation context are opaque.
//! A reassigned name callee captured in a different closure is also
//! opaque; the presence of any declaration assignment triggers that boundary.

use std::{
    borrow::Cow,
    collections::{HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    rc::Rc,
};

use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaCallExpr, LuaClosureExpr, LuaExpr, LuaFuncStat, LuaIndexExpr,
    LuaSyntaxId, LuaSyntaxNode, LuaVarExpr,
};
use rowan::TextSize;

use super::{SemanticModel, guards, index_access_path, path_write_affects, stable_access_path};
use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaClosureId};
use crate::{LuaMemberKey, LuaType};

type Access = (SemanticId, Vec<LuaMemberKey>);
type Bindings = Vec<(SemanticId, Vec<Access>)>;
// Histories contain each (callee, callsite, caller bindings) state at most once.
// This bounds recursion without merging replacement order or caller receivers.
#[derive(Clone, PartialEq, Eq)]
struct Invocation {
    closure: LuaClosureId,
    site: LuaSyntaxId,
    bindings: Rc<Bindings>,
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

fn receiver_targets(model: &SemanticModel, expected: &Access) -> Vec<Access> {
    let mut targets = vec![expected.clone()];
    if let Some(chunk) = model.chunk() {
        // Syntax fixes the finite set of roots and path lengths. Include callable
        // paths separately from the guarded-write target, so helper(methods) can
        // retain a method table unrelated to the guarded receiver.
        for index in chunk.descendants::<LuaIndexExpr>() {
            let expr = LuaExpr::IndexExpr(index);
            if let Some(path) =
                stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
                && !path.1.iter().any(LuaMemberKey::is_none)
                && !targets.contains(&path)
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
                if let Some(path) = LuaExpr::cast(var.syntax().clone()).and_then(|expr| {
                    stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
                }) {
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
                if !targets.contains(&child) {
                    targets.push(child.clone());
                }
                if let Some(value) = field.get_value_expr() {
                    tables.push((child, value));
                }
            }
        }
    }
    targets
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
    let Some(expected) =
        stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
    else {
        return false;
    };
    scan(model, &expected, node, after, before)
}

fn may_be_callee_value(value: LuaExpr) -> bool {
    guards::unparenthesized(value).is_some_and(|value| {
        matches!(
            value,
            LuaExpr::ClosureExpr(_) | LuaExpr::NameExpr(_) | LuaExpr::IndexExpr(_)
        )
    })
}

/// Inspect all direct writes in a visible body before expanding its callees.
/// Repeated callee/binding states terminate recursion. Only receiver prefixes
/// of either the guarded member or a fixed syntax path are retained, so descendant
/// arguments cannot create an unbounded sequence of distinct states.
fn scan(
    model: &SemanticModel,
    expected: &Access,
    node: &LuaSyntaxNode,
    after: TextSize,
    before: TextSize,
) -> bool {
    let cached = model
        .cache
        .borrow()
        .member_call_resolution
        .get(&model.file_id())
        .copied();
    let needs_members = cached.unwrap_or_else(|| {
        let present = model.chunk().is_some_and(|chunk| {
            chunk.descendants::<LuaCallExpr>().any(|call| {
                call.get_prefix_expr()
                    .and_then(guards::unparenthesized)
                    .is_some_and(|prefix| matches!(prefix, LuaExpr::IndexExpr(_)))
            }) || chunk
                .descendants::<LuaFuncStat>()
                .any(|func| matches!(func.get_func_name(), Some(LuaVarExpr::IndexExpr(_))))
                || chunk.descendants::<LuaAssignStat>().any(|assign| {
                    let (vars, values) = assign.get_var_and_expr_list();
                    vars.iter().zip(values).any(|(var, value)| {
                        matches!(var, LuaVarExpr::IndexExpr(_)) && may_be_callee_value(value)
                    })
                })
                || chunk
                    .descendants::<emmylua_parser::LuaTableField>()
                    .any(|field| field.get_value_expr().is_some_and(may_be_callee_value))
        });
        model
            .cache
            .borrow_mut()
            .member_call_resolution
            .insert(model.file_id(), present);
        present
    });
    let targets = if needs_members {
        receiver_targets(model, expected)
    } else {
        vec![expected.clone()]
    };
    let mut bodies = vec![(
        node.clone(),
        after,
        before,
        Bindings::new(),
        Invocations::new(),
        false,
    )];
    let mut seen = HashSet::new();
    while let Some((body, after, before, bindings, invocations, writes)) = bodies.pop() {
        let mut pending = vec![body];
        let mut calls = Vec::new();
        while let Some(node) = pending.pop() {
            if node.text_range().start() >= before
                || node.text_range().end() <= after
                || LuaClosureExpr::can_cast(node.kind().into())
            {
                continue;
            }
            if writes
                && node.text_range().end() <= before
                && let Some(assign) = LuaAssignStat::cast(node.clone())
                && assign
                    .get_var_and_expr_list()
                    .0
                    .iter()
                    .any(|var| write_conflicts(model, expected, var, &bindings))
            {
                return true;
            }
            if writes
                && node.text_range().end() <= before
                && let Some(func) = LuaFuncStat::cast(node.clone())
                && let Some(var) = func.get_func_name()
                && write_conflicts(model, expected, &var, &bindings)
            {
                return true;
            }
            if node.text_range().end() <= before
                && let Some(call) = LuaCallExpr::cast(node.clone())
            {
                if mutator_conflicts(model, expected, &call, &bindings) {
                    return true;
                }
                calls.push(call);
            }
            pending.extend(node.children());
        }
        for call in calls {
            for closure in local_callees(model, &call, &bindings, &targets, &invocations) {
                let Some(body) = closure.get_block() else {
                    continue;
                };
                let mut nested = bindings.clone();
                let mut args = call
                    .get_args_list()
                    .map(|args| args.get_args().collect::<Vec<_>>())
                    .unwrap_or_default();
                if call.is_colon_call()
                    && let Some(LuaExpr::IndexExpr(index)) = call.get_prefix_expr()
                    && let Some(receiver) = index.get_prefix_expr()
                {
                    args.insert(0, receiver);
                }
                let mut formals = Vec::new();
                if let Some(facts) = model.file_facts() {
                    formals.extend(
                        facts
                            .decls_by_owner_syntax(closure.get_syntax_id())
                            .filter(|decl| decl.is_implicit_self)
                            .map(|decl| decl.id.clone()),
                    );
                }
                if let Some(params) = closure.get_params_list() {
                    formals.extend(
                        params
                            .get_params()
                            .map(|param| SemanticId::decl(model.file_id(), param.get_range())),
                    );
                }
                for (index, id) in formals.into_iter().enumerate() {
                    let mut paths = args.get(index).map_or_else(Vec::new, |arg| {
                        access_paths(model, arg, &bindings, &targets)
                    });
                    paths.sort_by_cached_key(|path| (format!("{:?}", path.0), path.1.clone()));
                    paths.dedup();
                    nested.retain(|(bound, _)| *bound != id);
                    // Empty bindings identify omitted/scalar actuals and must not
                    // revive a previous recursive invocation's receiver.
                    nested.push((id, paths));
                }
                // These are same-file formal declarations, identified by position.
                nested.sort_by_key(|(id, _)| match id {
                    SemanticId::Decl(key) => Some(key.name_range.start()),
                    _ => None,
                });
                let mut nested_invocations = invocations.clone();
                let closure_id = LuaClosureId::from_node(closure.syntax());
                if needs_members {
                    let edge = Invocation::new(closure_id, call.get_syntax_id(), &bindings);
                    nested_invocations.retain(|entry| *entry != edge);
                    nested_invocations.push(edge);
                }
                if seen.insert((
                    closure.get_syntax_id(),
                    nested.clone(),
                    nested_invocations.clone(),
                )) {
                    bodies.push((
                        body.syntax().clone(),
                        TextSize::from(0),
                        body.get_range().end(),
                        nested,
                        nested_invocations,
                        true,
                    ));
                }
            }
        }
    }
    false
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
        let Some((root, mut keys)) =
            stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
        else {
            continue;
        };
        keys.extend(suffix);
        // Alias expansion can only prepend keys. A longer path cannot affect
        // this member, and pruning it makes recursive descendant aliases finite.
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

/// Return possible reaching assignments; `None` denotes the original declaration value.
fn reaching_values(
    model: &SemanticModel,
    root: &SemanticId,
    query: &LuaExpr,
) -> Vec<Option<LuaExpr>> {
    let Some(tree) = model.flow_tree() else {
        return vec![None];
    };
    if tree
        .latest_decl_assignment(root, query.get_position())
        .is_none()
    {
        return vec![None];
    }
    let Some(start) = tree.get_flow_id_at(query.get_position()) else {
        return vec![None];
    };
    let Some(chunk) = model.chunk() else {
        return vec![None];
    };
    let mut pending = vec![start];
    let mut visited = HashSet::new();
    let mut values = Vec::new();
    while let Some(flow) = pending.pop() {
        if !visited.insert(flow) {
            continue;
        }
        let Some(node) = tree.get_flow_node(flow) else {
            values.push(None);
            continue;
        };
        let replacement = tree.get_flow_effects(flow).iter().find_map(|effect| {
            if let FlowEffect::AssignDecl {
                decl, value_syntax, ..
            } = effect
                && decl == root
                && let Some(value) = value_syntax
                    .to_node_from_root(chunk.syntax())
                    .and_then(LuaExpr::cast)
                && value.get_range().end() < query.get_position()
            {
                Some(value)
            } else {
                None
            }
        });
        if let Some(value) = replacement {
            values.push(Some(value));
            continue;
        }
        if matches!(node.kind, FlowNodeKind::Unreachable) {
            continue;
        }
        match &node.antecedent {
            Some(FlowAntecedent::Single(previous)) => pending.push(*previous),
            Some(FlowAntecedent::Multiple(id)) => {
                if let Some(previous) = tree.get_multi_antecedents(*id) {
                    pending.extend(previous.iter().copied());
                } else {
                    values.push(None);
                }
            }
            None => values.push(None),
        }
    }
    values
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
    let Some(id @ SemanticId::Decl(_)) = model.resolve_name(name.get_position()) else {
        return Vec::new();
    };
    let SemanticId::Decl(key) = &id else {
        return Vec::new();
    };
    if key.file_id != model.file_id() {
        return Vec::new();
    }
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

/// Resolve member values along reaching paths, using invocation sites when a
/// callee's flow crosses its lexical entry. Unknown/ancestor writes stop that
/// path rather than reviving an older callable with the same member name.
fn member_values(
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
        if matches!(node.kind, FlowNodeKind::Start)
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
        if replaced || mutates || matches!(node.kind, FlowNodeKind::Unreachable) {
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
            LuaVarExpr::IndexExpr(_) => {
                stable_access_path(model, &expr).or_else(|| index_access_path(model, &expr))
            }
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

fn literal_member_key(key: emmylua_parser::LuaIndexKey) -> LuaMemberKey {
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
mod tests {
    use super::*;
    use crate::VirtualWorkspace;

    #[test]
    fn visible_callee_chains_preserve_receiver_identity() {
        for (argument, conflicts) in [("receiver", true), ("other", false)] {
            let mut ws = VirtualWorkspace::new();
            let source = format!(
                r#"
local receiver = {{value = 1}}
local other = {{value = 1}}
local function leaf(target) target.value = nil end
local function middle(target) leaf(target) end
local function outer(target) middle(target) end
outer({argument})
local observed = receiver.value
"#
            );
            let file = ws.def(&source);
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == "receiver.value")
                .unwrap();
            let call = chunk
                .descendants::<LuaCallExpr>()
                .find(|call| call.get_text() == format!("outer({argument})"))
                .unwrap();
            assert_eq!(
                calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
                conflicts
            );
        }
    }
    #[test]
    fn called_member_function_definitions_are_visible_writes() {
        for (receiver, conflicts) in [("receiver", true), ("other", false)] {
            let mut ws = VirtualWorkspace::new();
            let file = ws.def(&format!("local receiver = {{value = 1}}\nlocal other = {{}}\nlocal function replace() function {receiver}:value() end end\nreplace()\nlocal observed = receiver.value"));
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == "receiver.value")
                .unwrap();
            let call = chunk
                .descendants::<LuaCallExpr>()
                .find(|call| call.get_text() == "replace()")
                .unwrap();
            assert_eq!(
                calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
                conflicts
            );
        }
    }

    #[test]
    fn repeated_member_queries_through_visible_callees() {
        let mut source =
            "local receiver = {value = 1}\nlocal function step0(target) return target.value end\n"
                .to_string();
        for step in 1..64 {
            source.push_str(&format!(
                "local function step{step}(target) return step{}(target) end\n",
                step - 1
            ));
        }
        source.push_str("step63(receiver)\n");
        for query in 0..128 {
            source.push_str(&format!("local observed{query} = receiver.value\n"));
        }
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == "step63(receiver)")
            .unwrap();
        let reads = chunk
            .descendants::<LuaIndexExpr>()
            .filter(|index| index.get_text() == "receiver.value")
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 128);
        for read in reads {
            assert!(!calls_conflict(
                &model,
                &read,
                call.syntax(),
                call.get_range().end()
            ));
        }
    }

    #[test]
    fn mutable_captured_member_callees_without_invocation_are_opaque() {
        for (initial, replacement, conflicts) in [
            (
                "target.value = nil",
                "methods.clear = function() end",
                false,
            ),
            (
                "",
                "function methods.clear(target) target.value = nil end",
                false,
            ),
            ("target.value = nil", "", true),
        ] {
            let mut ws = VirtualWorkspace::new();
            let file = ws.def(&format!("local methods = {{}}\nfunction methods.clear(target) {initial} end\nlocal function check(receiver)\nmethods.clear(receiver)\nlocal observed = receiver.value\nend\n{replacement}\ncheck({{}})"));
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == "receiver.value")
                .unwrap();
            let call = chunk
                .descendants::<LuaCallExpr>()
                .find(|call| call.get_text() == "methods.clear(receiver)")
                .unwrap();
            assert_eq!(
                calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
                conflicts,
                "{initial}; {replacement}"
            );
        }
    }

    #[test]
    fn member_callee_presence_cache_separates_file_views() {
        let mut ws = VirtualWorkspace::new();
        let harmless = ws.def("local receiver = {value = 1}\nlocal function invoke() end\ninvoke()\nlocal value = receiver.value");
        let writer = ws.def("local receiver = {value = 1}\nlocal methods = {}\nfunction methods.invoke(target) target.value = nil end\nmethods.invoke(receiver)\nlocal value = receiver.value");
        let base = ws.analysis.semantic_model(harmless);
        for (file, call_text, expected) in [
            (harmless, "invoke()", false),
            (writer, "methods.invoke(receiver)", true),
            (harmless, "invoke()", false),
        ] {
            let model = base.model_for(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == "receiver.value")
                .unwrap();
            let call = chunk
                .descendants::<LuaCallExpr>()
                .find(|call| call.get_text() == call_text)
                .unwrap();
            assert_eq!(
                calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
                expected
            );
        }
    }

    #[test]
    fn invocation_fingerprints_preserve_semantic_equality() {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def("local function invoke(target) end");
        let model = ws.analysis.semantic_model(file);
        let closure = model
            .chunk()
            .unwrap()
            .descendants::<LuaClosureExpr>()
            .next()
            .unwrap();
        let id = SemanticId::decl(
            file,
            closure
                .get_params_list()
                .unwrap()
                .get_params()
                .next()
                .unwrap()
                .get_range(),
        );
        let paths = vec![(id.clone(), vec![(id.clone(), Vec::new())])];
        let left = Invocation::new(
            LuaClosureId::from_node(closure.syntax()),
            closure.get_syntax_id(),
            &paths,
        );
        let right = Invocation::new(left.closure, left.site, &paths);
        assert!(!Rc::ptr_eq(&left.bindings, &right.bindings));
        assert!(left == right);
        let hash = |entry: &Invocation| {
            let mut state = DefaultHasher::new();
            entry.hash(&mut state);
            state.finish()
        };
        assert_eq!(hash(&left), hash(&right));
        let mut collision = Invocation::new(left.closure, left.site, &vec![(id, Vec::new())]);
        collision.bindings_hash = left.bindings_hash;
        assert_eq!(hash(&left), hash(&collision));
        assert!(left != collision);
    }

    fn assert_known_call_conflicts(definitions: &str, call_text: &str) {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!("local receiver = {{value = 1}}\n{definitions}\n{call_text}\nlocal observed = receiver.value"));
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == call_text)
            .unwrap();
        assert!(calls_conflict(
            &model,
            &index,
            call.syntax(),
            call.get_range().end()
        ));
    }

    #[test]
    fn deep_known_callees_do_not_hide_conflicting_writes() {
        let mut definitions = "local methods = {}; function methods.unrelated() end\nlocal function step0(target) target.value = nil end\n".to_string();
        for step in 1..2050 {
            definitions.push_str(&format!(
                "local function step{step}(target) step{}(target) end\n",
                step - 1
            ));
        }
        assert_known_call_conflicts(&definitions, "step2049(receiver)");
    }

    #[test]
    fn wide_known_callees_do_not_hide_conflicting_writes() {
        let mut definitions = "local function write(target) target.value = nil end\n".to_string();
        for step in 0..2048 {
            definitions.push_str(&format!(
                "local function harmless{step}(target) return target end\n"
            ));
        }
        definitions.push_str("local function outer(target)\nwrite(target)\n");
        for step in 0..2048 {
            definitions.push_str(&format!("harmless{step}(target)\n"));
        }
        definitions.push_str("end");
        assert_known_call_conflicts(&definitions, "outer(receiver)");
    }

    #[test]
    fn long_callee_aliases_retain_visible_writes() {
        let mut definitions = "local function alias0(target) target.value = nil end\n".to_string();
        for alias in 1..40 {
            definitions.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
        }
        assert_known_call_conflicts(&definitions, "alias39(receiver)");
    }

    #[test]
    fn distant_callee_rebinding_retains_visible_writes() {
        let mut definitions = "local function writer(target) target.value = nil end\nlocal action = function() end\naction = writer\nlocal count = 0\n".to_string();
        for _ in 0..300 {
            definitions.push_str("count = count + 1\n");
        }
        assert_known_call_conflicts(&definitions, "action(receiver)");
    }

    #[test]
    fn long_formal_aliases_retain_visible_writes() {
        let mut definitions = "local other = {}\nlocal function outer(target)\nlocal alias0 = target\ntarget = other\n".to_string();
        for alias in 1..40 {
            definitions.push_str(&format!("local alias{alias} = alias{}\n", alias - 1));
        }
        definitions.push_str("alias39.value = nil\nend");
        assert_known_call_conflicts(&definitions, "outer(receiver)");
    }

    #[test]
    fn distant_formal_rebinding_retains_visible_writes() {
        let mut definitions =
            "local other = {}\nlocal function outer(target)\ntarget = receiver\nlocal count = 0\n"
                .to_string();
        for _ in 0..300 {
            definitions.push_str("count = count + 1\n");
        }
        definitions.push_str("target.value = nil\nend");
        assert_known_call_conflicts(&definitions, "outer(other)");
    }

    #[test]
    fn detached_member_captures_do_not_target_the_replacement() {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(
            "local receiver = {child = {value = 1}}\nlocal saved = receiver.child\nreceiver.child = {value = 2}\nlocal function write(target) target.value = nil end\nwrite(saved)\nlocal observed = receiver.child.value",
        );
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let index = chunk
            .descendants::<LuaIndexExpr>()
            .find(|index| index.get_text() == "receiver.child.value")
            .unwrap();
        let call = chunk
            .descendants::<LuaCallExpr>()
            .find(|call| call.get_text() == "write(saved)")
            .unwrap();
        assert!(!calls_conflict(
            &model,
            &index,
            call.syntax(),
            call.get_range().end()
        ));
    }
}

#[cfg(test)]
mod rawset_key_tests {
    use super::*;
    use crate::VirtualWorkspace;

    #[test]
    fn numeric_rawset_keys_keep_receiver_and_ancestor_identity() {
        for (guarded, call_text, conflicts) in [
            ("elem[1].handler", "rawset(elem, 1.0, nil)", true),
            ("elem[1].handler", "rawset(elem, 1.5, nil)", false),
            ("elem[2].handler", "rawset(elem, 1.0, nil)", false),
            ("elem[1].handler", "rawset(other, 1.0, nil)", false),
            ("elem.handler", "rawset(elem, dynamic, nil)", true),
            ("elem.handler", "rawset(elem, mixed, nil)", true),
            ("elem.handler", "rawset(elem, numeric, nil)", false),
            ("elem[1]", "rawset(elem, numeric, nil)", true),
            ("elem.child.handler", "rawset(elem, 'child', nil)", true),
            ("elem.child.handler", "rawset(elem.child, 1.5, nil)", false),
            ("elem[1].handler", "rawset(elem[1], 'handler', nil)", true),
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            let file = ws.def(&format!("---@param numeric number\n---@param mixed number|string\nlocal function inspect(elem, other, numeric, dynamic, mixed)\n{call_text}\nlocal observed = {guarded}\nend"));
            let model = ws.analysis.semantic_model(file);
            let chunk = model.chunk().unwrap();
            let index = chunk
                .descendants::<LuaIndexExpr>()
                .find(|index| index.get_text() == guarded)
                .unwrap();
            let call = chunk
                .descendants::<LuaCallExpr>()
                .find(|call| call.get_text() == call_text)
                .unwrap();
            assert_eq!(
                calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
                conflicts,
                "{guarded}; {call_text}"
            );
        }
    }
}
