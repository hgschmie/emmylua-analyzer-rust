//! Condition facts for stable computed accesses, which have no static member ID.
//! Call effects follow visible same-file callees; opaque/inter-file calls preserve
//! guards unless a known conflicting write is visible. This is not a purity proof.

use emmylua_parser::{
    BinaryOperator, LuaAssignStat, LuaAstNode, LuaExpr, LuaIndexExpr, LuaIndexKey, LuaLiteralToken,
    UnaryOperator,
};
use rowan::TextSize;

use super::{SemanticModel, computed_effects};
use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowId, FlowNodeKind, LuaClosureId};
use crate::{LuaMemberKey, LuaType};

// Bound branching guard proofs; exhaustion declines proof and may retain a nil warning.
const MAX_GUARD_PROOF_STEPS: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Part {
    Member(LuaMemberKey),
    Key(Box<Access>),
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Access {
    pub(super) root: SemanticId,
    pub(super) parts: Vec<Part>,
}

impl Access {
    pub(super) fn depends_on_globals(&self) -> bool {
        matches!(self.root, SemanticId::Name(_))
            || self
                .parts
                .iter()
                .any(|part| matches!(part, Part::Key(key) if key.depends_on_globals()))
    }

    pub(super) fn depends_on(&self, decl: &SemanticId) -> bool {
        &self.root == decl
            || self
                .parts
                .iter()
                .any(|part| matches!(part, Part::Key(key) if key.depends_on(decl)))
    }
}

pub(super) fn guarded_type(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    offset: TextSize,
) -> Option<LuaType> {
    // Static members already have assignment-aware flow; do not replace their
    // more precise value with the collection's declared element contract.
    if model
        .resolve_member(index)
        .is_some_and(|member| member.member_id.is_some())
    {
        return None;
    }
    let access = access(model, &LuaExpr::IndexExpr(index.clone()))?;
    let prefix = index.get_prefix_expr()?;
    let owner_type = model.type_of_expr_at(prefix.get_syntax_id(), offset);
    // Explicit object fields keep upstream's richer callback and builtin-alias guards.
    if let LuaType::Object(object) = &owner_type
        && (object.get_index_access().is_empty()
            || matches!(access.parts.last(), Some(Part::Member(key)) if object.get_fields().contains_key(key)))
    {
        return None;
    }
    // A computed key can select declared class fields as well as collection entries.
    // Explicit __index handlers can change values between reads and remain excluded.
    match &owner_type {
        LuaType::Array(_) | LuaType::TableGeneric(_) => {}
        LuaType::Generic(generic)
            if generic.get_base_type_id() == crate::LuaTypeDeclId::global("table")
                && model
                    .type_def_of(&generic.get_base_type_id())
                    .is_some_and(|def| model.is_std_file(def.file_id)) => {}
        _ if named_collection(model, &owner_type, CollectionUse::ReadGuard) => {}
        _ => return None,
    }
    // This is the raw expression cache, distinct from the flow-sensitive query
    // that called us. Recursive raw inference can still yield Unknown; only a
    // concrete owner array contract supplies a conservative element fallback.
    let inferred = model.type_of_expr(index.get_syntax_id());
    let base = match (&inferred, &owner_type) {
        // Recursive element inference can be unknown at a loop/write site. The
        // owner's array contract is still available, as in guarded_index_type.
        (LuaType::Unknown, LuaType::Array(array)) => {
            LuaType::from_vec(vec![array.get_base().clone(), LuaType::Nil])
        }
        _ => inferred,
    };
    // GuardSearch walks backwards in evaluation order. A conflicting call before
    // the guard was established must not erase that newer guard; intervening
    // conditions and assignment effects still invalidate it during the walk.
    let closure = LuaClosureId::from_node(index.syntax());
    let current = model.flow_tree()?.get_flow_id_at(offset)?;
    let mut search = GuardSearch::new(model, &access, offset, closure);
    let narrowed = search.narrow(current, base.clone());
    // A raw dictionary value contract can omit absence, even when its read
    // projection is nullable. Preserve an explicit non-nil proof in that case.
    (narrowed != base
        || (!base.is_nullable()
            && !matches!(base, LuaType::Any | LuaType::Unknown | LuaType::Never)
            && search.prove(current)))
    .then_some(narrowed)
}

// Search all reaching paths, memoizing joins so nested branches do not duplicate
// work. A cycle or exhausted budget cannot establish a guard.
struct GuardSearch<'a, 'db> {
    model: &'a SemanticModel<'db>,
    access: &'a Access,
    offset: TextSize,
    closure: LuaClosureId,
    initializer: Option<FlowId>,
    // Both proof modes share one query budget. Exhaustion may lose a valid
    // narrowing but must never manufacture a proof; it bounds their total work.
    remaining: usize,
    memo: hashbrown::HashMap<FlowId, bool>,
    typed_memo: hashbrown::HashMap<(FlowId, LuaType), Option<LuaType>>,
}

impl<'a, 'db> GuardSearch<'a, 'db> {
    fn new(
        model: &'a SemanticModel<'db>,
        access: &'a Access,
        offset: TextSize,
        closure: LuaClosureId,
    ) -> Self {
        Self {
            model,
            access,
            offset,
            closure,
            initializer: None,
            remaining: MAX_GUARD_PROOF_STEPS,
            memo: hashbrown::HashMap::new(),
            typed_memo: hashbrown::HashMap::new(),
        }
    }

    fn narrow(&mut self, current: FlowId, base: LuaType) -> LuaType {
        let key = (current, base.clone());
        if let Some(result) = self.typed_memo.get(&key) {
            return result.clone().unwrap_or(base);
        }
        if self.remaining == 0 {
            return base;
        }
        self.remaining -= 1;
        // A backedge cannot establish an older fact. Include incoming filters
        // so a shared predecessor cannot conflate different reaching branches.
        self.typed_memo.insert(key.clone(), None);
        let result = self.narrow_node(current, base.clone()).unwrap_or(base);
        self.typed_memo.insert(key, Some(result.clone()));
        result
    }

    fn condition_invalidates(&self, condition: &LuaExpr) -> bool {
        LuaClosureId::from_node(condition.syntax()) != self.closure
            || computed_effects::calls_conflict(
                self.model,
                self.access,
                condition.syntax(),
                self.offset,
            )
    }

    fn assignment_invalidates(&self, assignment: &LuaAssignStat) -> bool {
        assignment.get_range().end() <= self.offset
            && (LuaClosureId::from_node(assignment.syntax()) != self.closure
                || computed_effects::calls_conflict(
                    self.model,
                    self.access,
                    assignment.syntax(),
                    self.offset,
                )
                || assignment.get_var_and_expr_list().0.iter().any(|var| {
                    computed_effects::write_conflicts(self.model, self.access, var, self.offset)
                }))
    }

    fn narrow_node(&mut self, current: FlowId, mut base: LuaType) -> Option<LuaType> {
        let model = self.model;
        let tree = model.flow_tree()?;
        let chunk = model.chunk()?;
        let node = tree.get_flow_node(current)?;
        let offset = self.offset;
        let access = self.access;
        match &node.kind {
            FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                let condition = ptr.to_node(&chunk)?;
                if self.condition_invalidates(&condition) {
                    return Some(base);
                }
                if condition.get_range().end() <= offset {
                    let truthy = matches!(node.kind, FlowNodeKind::TrueCondition(_));
                    if let Some((target, primitive, keep_matching)) =
                        super::primitive_guard_target(model, &condition, truthy)
                        && self.matches_guard(&target, current)
                    {
                        base =
                            super::filter_type_by_primitive(model, base, &primitive, keep_matching);
                    } else if excludes_nil(&condition, truthy, &|expr| {
                        self.matches_guard(expr, current)
                    }) {
                        base = super::remove_type(base, &LuaType::Nil);
                    }
                }
            }
            FlowNodeKind::Assignment(ptr) => {
                let assignment = ptr.to_node(&chunk)?;
                if self.assignment_invalidates(&assignment) {
                    return Some(base);
                }
            }
            FlowNodeKind::CallExprStat(ptr) => {
                if computed_effects::calls_conflict(
                    model,
                    access,
                    ptr.to_node(&chunk)?.syntax(),
                    offset,
                ) {
                    return Some(base);
                }
            }
            // Goto joins and function installations can change reaching values;
            // this bounded proof does not model those effects across the boundary.
            FlowNodeKind::NamedLabel(_) | FlowNodeKind::ImplFunc(_) => return Some(base),
            FlowNodeKind::Unreachable => return Some(LuaType::Never),
            _ => {}
        }
        for effect in tree.get_flow_effects(current) {
            if let FlowEffect::LoopEntry(syntax) = effect {
                let loop_node = syntax.to_node_from_root(chunk.syntax())?;
                if computed_effects::loop_conflicts(model, access, &loop_node) {
                    return Some(base);
                }
            }
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                if value_syntax.get_range().end() <= offset && access.depends_on(decl) {
                    return Some(base);
                }
                let value = value_syntax.to_node_from_root(chunk.syntax())?;
                if computed_effects::calls_conflict(model, access, &value, offset) {
                    return Some(base);
                }
            }
        }
        match node.antecedent.as_ref() {
            Some(FlowAntecedent::Single(next)) => Some(self.narrow(*next, base)),
            Some(FlowAntecedent::Multiple(id)) => {
                let predecessors = tree.get_multi_antecedents(*id)?;
                if predecessors.is_empty() {
                    return Some(base);
                }
                Some(predecessors.iter().fold(LuaType::Never, |merged, id| {
                    super::merge_types(merged, self.narrow(*id, base.clone()))
                }))
            }
            None => Some(base),
        }
    }

    fn prove(&mut self, current: FlowId) -> bool {
        if let Some(result) = self.memo.get(&current) {
            return *result;
        }
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        // An in-progress node is false: following a loop backedge must not prove
        // its own precondition. Completed shared predecessors are reused.
        self.memo.insert(current, false);
        let result = self.visit(current).unwrap_or(false);
        self.memo.insert(current, result);
        result
    }

    fn visit(&mut self, current: FlowId) -> Option<bool> {
        let model = self.model;
        let tree = model.flow_tree()?;
        let chunk = model.chunk()?;
        let node = tree.get_flow_node(current)?;
        let offset = self.offset;
        let access = self.access;
        match &node.kind {
            FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                let condition = ptr.to_node(&chunk)?;
                if self.condition_invalidates(&condition) {
                    return Some(false);
                }
                if self.initializer.is_none()
                    && condition.get_range().end() <= offset
                    && (excludes_nil(
                        &condition,
                        matches!(node.kind, FlowNodeKind::TrueCondition(_)),
                        &|expr| self.matches_guard(expr, current),
                    ) || super::primitive_guard_target(
                        model,
                        &condition,
                        matches!(node.kind, FlowNodeKind::TrueCondition(_)),
                    )
                    .is_some_and(|(target, primitive, keep)| {
                        self.matches_guard(&target, current)
                            && super::filter_type_by_primitive(
                                model,
                                LuaType::Nil,
                                &primitive,
                                keep,
                            ) == LuaType::Never
                    }))
                {
                    return Some(true);
                }
            }
            FlowNodeKind::Assignment(ptr) => {
                let assignment = ptr.to_node(&chunk)?;
                if self.assignment_invalidates(&assignment) {
                    return Some(false);
                }
            }
            FlowNodeKind::CallExprStat(ptr) => {
                if computed_effects::calls_conflict(
                    model,
                    access,
                    ptr.to_node(&chunk)?.syntax(),
                    offset,
                ) {
                    return Some(false);
                }
            }
            FlowNodeKind::NamedLabel(_) | FlowNodeKind::ImplFunc(_) => return Some(false),
            FlowNodeKind::Unreachable => return Some(true),
            _ => {}
        }
        for effect in tree.get_flow_effects(current) {
            if let FlowEffect::LoopEntry(syntax) = effect {
                let loop_node = syntax.to_node_from_root(chunk.syntax())?;
                if computed_effects::loop_conflicts(model, access, &loop_node) {
                    return Some(false);
                }
            }
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                if value_syntax.get_range().end() <= offset && access.depends_on(decl) {
                    return Some(false);
                }
                let value = value_syntax.to_node_from_root(chunk.syntax())?;
                if computed_effects::calls_conflict(model, access, &value, offset) {
                    return Some(false);
                }
            }
        }
        // Check effects first: a sibling initializer may change the lookup key.
        if self.initializer == Some(current) {
            return Some(true);
        }
        match node.antecedent.as_ref()? {
            FlowAntecedent::Single(next) => Some(self.prove(*next)),
            FlowAntecedent::Multiple(id) => {
                let predecessors = tree.get_multi_antecedents(*id)?;
                Some(!predecessors.is_empty() && predecessors.iter().all(|id| self.prove(*id)))
            }
        }
    }

    fn matches_guard(&self, expr: &LuaExpr, current: FlowId) -> bool {
        if access(self.model, expr).as_ref() == Some(self.access) {
            return true;
        }
        self.alias_is_current(expr, current).unwrap_or(false)
    }

    fn alias_is_current(&self, expr: &LuaExpr, current: FlowId) -> Option<bool> {
        let LuaExpr::NameExpr(name) = expr else {
            return None;
        };
        let model = self.model;
        let id = model.resolve_name(name.get_position())?;
        let facts = model.file_facts()?;
        let decl = facts.decl_by_id(&id)?;
        let tree = model.flow_tree()?;
        // Only stable locals have a retained value. Reassignment or deferred
        // execution must not turn a saved reference into a fresh lookup fact.
        if !decl.kind.is_local() || tree.has_decl_assignment(&id) {
            return None;
        }
        let chunk = model.chunk()?;
        let value = LuaExpr::cast(decl.value_expr_syntax?.to_node_from_root(chunk.syntax())?)?;
        if LuaClosureId::from_node(value.syntax()) != self.closure
            || access(model, &value).as_ref() != Some(self.access)
        {
            return None;
        }
        let initializer = tree.decl_position_flow(decl.owner_syntax?.get_range().start())?;
        let mut search = Self::new(model, self.access, self.offset, self.closure);
        search.initializer = Some(initializer);
        // Also prove the source stayed unchanged between capturing the alias
        // and testing it; checking only effects after the guard is insufficient.
        Some(search.prove(current))
    }
}

// Both guard and assignment facts require ordinary collection semantics.
pub(super) fn ordinary_collection(model: &SemanticModel, owner_type: &LuaType) -> bool {
    match &owner_type {
        LuaType::Array(_) | LuaType::TableGeneric(_) => true,
        LuaType::Object(object) => {
            !object.get_index_access().is_empty()
                && !object
                    .get_fields()
                    .keys()
                    .any(|key| intercepts_access(key, CollectionUse::Assignment))
        }
        LuaType::Generic(generic)
            if generic.get_base_type_id() == crate::LuaTypeDeclId::global("table")
                && model
                    .type_def_of(&generic.get_base_type_id())
                    .is_some_and(|def| model.is_std_file(def.file_id)) =>
        {
            true
        }
        _ => named_collection(model, owner_type, CollectionUse::Assignment),
    }
}

// Collection classification is a proof, so partial inherited contracts or
// member enumeration must never enable a guard or assignment fact.
// Declared fields admit computed reads only: the caller still needs a concrete
// value contract and a reaching guard. This predicate does not prove presence.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CollectionUse {
    ReadGuard,
    Assignment,
}

fn intercepts_access(key: &LuaMemberKey, purpose: CollectionUse) -> bool {
    matches!(key, LuaMemberKey::Name(name) if name == "__index" || (purpose == CollectionUse::Assignment && name == "__newindex"))
}

fn named_collection(model: &SemanticModel, owner: &LuaType, purpose: CollectionUse) -> bool {
    let _query = crate::semantic_model::cache::QueryCacheScope::new(&model.cache);
    let revision = model.cache.borrow().revision();
    let contracts = crate::semantic_model::inherited_index::dictionaries(model, owner);
    if !model.cache.borrow().is_complete(revision) {
        return false;
    }
    let members = model.member_infos(owner);
    let collection = (!contracts.is_empty()
        || (purpose == CollectionUse::ReadGuard
            && members.iter().any(|member| member.id.is_some())))
        && !members
            .iter()
            .any(|member| intercepts_access(&member.key, purpose));
    collection && model.cache.borrow().is_complete(revision)
}

// Globals use their binding name rather than a particular declaration site.
// Locals retain declaration identity, keeping shadowed names separate.
pub(super) fn reference(
    model: &SemanticModel,
    name: &emmylua_parser::LuaNameExpr,
) -> Option<SemanticId> {
    if let Some(decl) = model.resolve_name(name.get_position())
        && model
            .file_facts()?
            .decl_by_id(&decl)
            .is_some_and(|decl| decl.kind.is_local())
    {
        return Some(decl);
    }
    // A custom lexical environment can change the meaning of a global name.
    if model
        .file_facts()?
        .find_visible_decl_before_offset("_ENV", name.get_position())
        .is_some()
    {
        return None;
    }
    Some(SemanticId::name(name.get_name_text()?.into()))
}

pub(super) fn access(model: &SemanticModel, expr: &LuaExpr) -> Option<Access> {
    match expr {
        LuaExpr::NameExpr(name) => Some(Access {
            root: reference(model, name)?,
            parts: Vec::new(),
        }),
        LuaExpr::IndexExpr(index) => {
            let mut result = access(model, &index.get_prefix_expr()?)?;
            let part = match index.get_index_key()? {
                LuaIndexKey::Expr(expr) => Part::Key(Box::new(access(model, &expr)?)),
                key => {
                    let key = super::member_call_effects::literal_member_key(key);
                    if key.is_none() {
                        return None;
                    }
                    Part::Member(key)
                }
            };
            result.parts.push(part);
            Some(result)
        }
        LuaExpr::ParenExpr(paren) => access(model, &paren.get_expr()?),
        _ => None,
    }
}

fn excludes_nil(expr: &LuaExpr, truthy: bool, matches: &impl Fn(&LuaExpr) -> bool) -> bool {
    if matches(expr) {
        return truthy;
    }
    match expr {
        LuaExpr::ParenExpr(paren) => paren
            .get_expr()
            .is_some_and(|inner| excludes_nil(&inner, truthy, matches)),
        LuaExpr::UnaryExpr(unary)
            if unary
                .get_op_token()
                .is_some_and(|op| op.get_op() == UnaryOperator::OpNot) =>
        {
            unary
                .get_expr()
                .is_some_and(|inner| excludes_nil(&inner, !truthy, matches))
        }
        LuaExpr::BinaryExpr(binary) => {
            let Some((left, right)) = binary.get_exprs() else {
                return false;
            };
            let Some(op) = binary.get_op_token() else {
                return false;
            };
            let is_nil = |expr: &LuaExpr| matches!(expr, LuaExpr::LiteralExpr(lit) if matches!(lit.get_literal(), Some(LuaLiteralToken::Nil(_))));
            match op.get_op() {
                BinaryOperator::OpEq | BinaryOperator::OpNe => {
                    let non_nil = (op.get_op() == BinaryOperator::OpNe) == truthy;
                    non_nil
                        && ((is_nil(&left) && matches(&right))
                            || (is_nil(&right) && matches(&left)))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod class_field_tests;

#[cfg(test)]
mod effect_boundary_tests;

#[cfg(test)]
mod alias_merge_tests;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod expression_order_tests;

#[cfg(test)]
mod numeric_access_tests;
