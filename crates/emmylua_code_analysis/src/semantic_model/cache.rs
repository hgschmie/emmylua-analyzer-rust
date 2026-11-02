//! Request-local cache shared across `SemanticModel` file views.
//!
//! A root model and its `model_for` views share one cache. Recursion state is
//! represented by `CacheEntry::InProgress` so re-entry checks stay O(1).
//! Each root model borrows an immutable database and creates a fresh cache. File
//! edits require a new model; the incompleteness revision below is a query-state
//! counter, not a file revision. Syntax-only facts can live for the whole model.

use hashbrown::{HashMap, HashSet};
use std::cell::Cell;
use std::rc::Rc;

use emmylua_parser::{LuaExpr, LuaSyntaxId};
use rowan::TextSize;
use smol_str::SmolStr;

use crate::member_key::LuaMemberKey;
use crate::semantic_db::MemberList;
use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::FlowId;
use crate::{FileId, LuaFunctionType, LuaType, LuaTypeDeclId, LuaTypeNode};

use super::flow::member_call_effects::{CalleeCacheKey, ReceiverTargets};
use super::infer::unify::TplBindings;
use super::member::MemberInfo;
use super::{CallSiteAnalysis, ResolvedMember};
use crate::IndexInfo;

/// Identifies an outer scan, which always starts with empty bindings and invocation history.
/// Nested callees are expanded inside that scan with their substituted context;
/// they use `CalleeCacheKey` and never re-enter this cache under those bindings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct MemberCallEffectKey {
    pub(super) file: FileId,
    pub(super) receiver: SemanticId,
    pub(super) path: Vec<LuaMemberKey>,
    pub(super) body: LuaSyntaxId,
    pub(super) after: TextSize,
    pub(super) before: TextSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum BuiltinKey {
    Expression(LuaSyntaxId),
    Flow(SemanticId, FlowId),
}

/// Keep provisional memo entries while dependent queries still need them.
/// Discard them at the outer query boundary so the next query can retry.
#[must_use = "keep the scope alive until the query finishes"]
pub(super) struct QueryCacheScope<'a> {
    cache: &'a super::LazyCache,
    revision: u64,
}

impl<'a> QueryCacheScope<'a> {
    pub(super) fn new(cache: &'a super::LazyCache) -> Self {
        let mut state = cache.borrow_mut();
        if state.query_depth == 0 {
            state.evaluation_work = 0;
            state.evaluation_exhausted = false;
            state.evaluation_origin = None;
        }
        state.query_depth += 1;
        Self {
            cache,
            revision: state.revision(),
        }
    }
}

/// Share the resource budget across alias evaluation and type comparisons.
/// Queue-driven recursion uses the work budget. Nested semantic queries also
/// have a stack guard. Exhaustion is reported separately from type mismatch.
pub(super) struct TypeEvaluationScope<'a> {
    query: QueryCacheScope<'a>,
}

impl<'a> TypeEvaluationScope<'a> {
    pub(super) const MAX_TYPE_VISITS: usize = 1_000_000;
    pub(super) const MAX_EVALUATION_DEPTH: usize = 64;

    /// Enter a nested evaluation sharing the enclosing query's remaining work.
    /// Exhausted work or excessive semantic nesting records an incomplete
    /// evaluation before returning `None`; a fresh outer query resets the budget.
    pub(super) fn new(cache: &'a super::LazyCache) -> Option<Self> {
        if cache.is_cancelled() {
            cache.borrow().mark_incomplete();
            return None;
        }
        let query = QueryCacheScope::new(cache);
        let mut state = cache.borrow_mut();
        if state.evaluation_depth >= Self::MAX_EVALUATION_DEPTH || state.evaluation_exhausted {
            state.exhaust_evaluation();
            return None;
        }
        state.evaluation_depth += 1;
        Some(Self { query })
    }

    /// Charge the types inspected or rebuilt by one evaluator operation.
    pub(super) fn visit(&self, ty: &LuaType) -> bool {
        let mut cache = self.query.cache.borrow_mut();
        if cache.evaluation_origin.is_none() {
            cache.evaluation_origin = match ty {
                LuaType::Ref(id) | LuaType::Def(id) => Some(id.clone()),
                LuaType::Generic(generic) => Some(generic.get_base_type_id()),
                _ => None,
            };
        }
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            if self.query.cache.is_cancelled() {
                cache.mark_incomplete();
                return false;
            }
            if cache.evaluation_exhausted || cache.evaluation_work >= Self::MAX_TYPE_VISITS {
                cache.exhaust_evaluation();
                return false;
            }
            cache.evaluation_work += 1;
            ty.push_direct_children(&mut pending);
        }
        true
    }
}

impl Drop for TypeEvaluationScope<'_> {
    fn drop(&mut self) {
        self.query.cache.borrow_mut().evaluation_depth -= 1;
    }
}

impl Drop for QueryCacheScope<'_> {
    fn drop(&mut self) {
        let mut cache = self.cache.borrow_mut();
        cache.query_depth -= 1;
        if cache.query_depth == 0 && !cache.is_complete(self.revision) {
            cache.discard_provisional();
        }
    }
}

/// Remove an unfinished constructor entry even when its callback unwinds.
pub(super) struct ConstructorQueryGuard<'a> {
    cache: &'a super::LazyCache,
    key: (FileId, LuaSyntaxId, LuaType),
}

impl<'a> ConstructorQueryGuard<'a> {
    pub(super) fn new(cache: &'a super::LazyCache, key: (FileId, LuaSyntaxId, LuaType)) -> Self {
        cache
            .borrow_mut()
            .constructor_diagnostics
            .insert(key.clone(), CacheEntry::InProgress);
        Self { cache, key }
    }
}

impl Drop for ConstructorQueryGuard<'_> {
    fn drop(&mut self) {
        let mut cache = self.cache.borrow_mut();
        if matches!(
            cache.constructor_diagnostics.get(&self.key),
            Some(CacheEntry::InProgress)
        ) {
            cache.constructor_diagnostics.remove(&self.key);
            cache.mark_incomplete();
        }
    }
}

/// An active constructor query may be suspended while a child query completes.
/// Drop the active-key guard before the outer query scope, including during unwind.
pub(crate) struct ConstructorQuery<'a> {
    active: ConstructorQueryGuard<'a>,
    _scope: QueryCacheScope<'a>,
    revision: u64,
}

impl<'a> ConstructorQuery<'a> {
    pub(super) fn new(cache: &'a super::LazyCache, key: (FileId, LuaSyntaxId, LuaType)) -> Self {
        let scope = QueryCacheScope::new(cache);
        let revision = cache.borrow().revision();
        Self {
            active: ConstructorQueryGuard::new(cache, key),
            _scope: scope,
            revision,
        }
    }

    pub(crate) fn finish(
        self,
        result: Option<Vec<crate::check::checker::ConstructorDiagnostic>>,
    ) -> Option<Vec<crate::check::checker::ConstructorDiagnostic>> {
        let mut cache = self.active.cache.borrow_mut();
        let Some(result) = result else {
            cache.mark_incomplete();
            cache.constructor_diagnostics.remove(&self.active.key);
            return None;
        };
        if cache.is_complete(self.revision) {
            cache
                .constructor_diagnostics
                .insert(self.active.key.clone(), CacheEntry::Ready(result.clone()));
        } else {
            cache.constructor_diagnostics.remove(&self.active.key);
        }
        Some(result)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CacheEntry<T> {
    InProgress,
    Ready(T),
    /// An unfinished result reused only within the current outer query.
    Provisional(T),
}

impl<T> CacheEntry<T> {
    fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    pub(crate) fn finish(value: T, complete: bool) -> Self {
        if complete {
            Self::Ready(value)
        } else {
            Self::Provisional(value)
        }
    }

    /// An unfinished comparison must not become proof of either outcome.
    pub(crate) fn get_complete<'a>(&'a self, cache: &SemanticLocalCache) -> Option<&'a T> {
        if let Self::Ready(value) = self {
            Some(value)
        } else {
            cache.mark_incomplete();
            None
        }
    }

    pub(crate) fn get<'a>(&'a self, cache: &SemanticLocalCache) -> Option<&'a T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::InProgress => {
                cache.mark_incomplete();
                None
            }
            Self::Provisional(value) => {
                cache.mark_incomplete();
                Some(value)
            }
        }
    }

    /// Base expression/declaration/member inference uses empty recursive reads
    /// as seeds (for example, `value = value or {}`). Preserve that rule while
    /// propagating provisional results from dependent semantic queries.
    pub(crate) fn get_inference_seed<'a>(&'a self, cache: &SemanticLocalCache) -> Option<&'a T> {
        if matches!(self, Self::InProgress) {
            None
        } else {
            self.get(cache)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TraceSubject {
    Decl {
        decl: SemanticId,
        guards: bool,
        casts: bool,
        assignments: bool,
        merge_branch: bool,
    },
    Member {
        member: SemanticId,
        access: Option<LuaSyntaxId>,
    },
}

/// Memo key for a flow trace, including its incoming guard/cast context.
/// Equivalent path operations are normalized before constructing this key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TraceStateKey {
    pub(crate) file_id: FileId,
    pub(crate) subject: TraceSubject,
    pub(crate) flow_id: u32,
    pub(crate) path: super::flow::PathState,
}

#[derive(Default)]
pub(crate) struct TypeDefCache {
    /// None 表示尚未计算, Some 空列表表示已知没有父类型.
    pub(crate) base_types: Option<Vec<LuaType>>,
}

/// Syntax-only cast positions and asserted expressions in one file view.
#[derive(Default)]
pub(crate) struct LocalCallableCasts {
    pub(crate) first_cast: HashMap<SemanticId, TextSize>,
    pub(crate) asserted_exprs: HashSet<LuaSyntaxId>,
}

#[derive(Default)]
pub(crate) struct SemanticLocalCache {
    /// Unfiltered constructor failures shared by all diagnostic passes in this model.
    pub(crate) constructor_diagnostics: HashMap<
        (FileId, LuaSyntaxId, LuaType),
        CacheEntry<Vec<crate::check::checker::ConstructorDiagnostic>>,
    >,
    /// Changes whenever a query reads an unfinished dependency. Most queries
    /// use this to decide whether a result can be cached. Type comparisons
    /// track completeness in their result, including discarded alternatives.
    incomplete_revision: Cell<u64>,
    query_depth: usize,
    evaluation_depth: usize,
    evaluation_work: usize,
    pub(crate) evaluation_exhausted: bool,
    evaluation_origin: Option<LuaTypeDeclId>,
    pub(crate) evaluation_errors: HashSet<Option<LuaTypeDeclId>>,
    /// Closure returns depend on the VM environment: track active queries only.
    pub(crate) closure_return_in_progress: HashSet<(FileId, LuaSyntaxId)>,
    pub(crate) loop_carried:
        HashMap<(FileId, LuaSyntaxId, SemanticId), CacheEntry<Option<LuaType>>>,
    /// Completed syntax/CFG reachability within one immutable model snapshot.
    /// File edits create a fresh model/cache, so FileId needs no revision here.
    pub(crate) loop_statement_reachable: HashMap<FileId, Rc<Vec<bool>>>,
    /// Syntax/name-identity facts are stable for the shared model snapshot.
    pub(crate) loop_rhs_dependencies: HashMap<(FileId, LuaSyntaxId, SemanticId), bool>,
    pub(crate) expr_type: HashMap<(FileId, LuaSyntaxId), CacheEntry<LuaType>>,
    pub(crate) decl_type: HashMap<(FileId, SemanticId), CacheEntry<Option<LuaType>>>,
    pub(crate) member_type: HashMap<(FileId, SemanticId), CacheEntry<Option<LuaType>>>,
    pub(crate) resolve_member: HashMap<(FileId, LuaSyntaxId), Option<ResolvedMember>>,
    pub(crate) expr_type_at: HashMap<(FileId, LuaSyntaxId, TextSize), CacheEntry<LuaType>>,
    /// Cast-bearing flow bindings on the node and its ancestors, nearest first.
    pub(crate) expr_cast_flows: HashMap<(FileId, LuaSyntaxId), Rc<Vec<FlowId>>>,
    /// Syntax-only member-callee presence for this model snapshot; never call effects.
    pub(crate) member_call_resolution: HashMap<FileId, bool>,
    /// Fixed targets are isolated per file when model views share this query cache.
    pub(super) member_receiver_targets: HashMap<FileId, CacheEntry<ReceiverTargets>>,
    pub(super) member_call_effects: HashMap<MemberCallEffectKey, CacheEntry<bool>>,
    pub(super) member_callees:
        HashMap<CalleeCacheKey, CacheEntry<Vec<emmylua_parser::LuaClosureExpr>>>,
    pub(crate) resolve_name: HashMap<(FileId, TextSize), Option<SemanticId>>,
    /// Builtin identity at each read and declaration/flow state.
    pub(crate) builtin_function: HashMap<(FileId, BuiltinKey), Option<SmolStr>>,
    /// Whether all writes to a local occur in its own closure.
    pub(crate) builtin_local_writes: HashMap<(FileId, SemanticId), bool>,
    pub(crate) resolve_owner_set: HashMap<SemanticId, Vec<SemanticId>>,
    pub(crate) member_type_at: HashMap<(FileId, SemanticId, TextSize), LuaType>,
    pub(crate) flow_decl: HashMap<(FileId, SemanticId, FlowId), LuaType>,
    /// Whole-function, syntax-only check for the local-table last-element proof.
    pub(crate) array_length_stable: HashMap<(FileId, SemanticId), bool>,
    /// Values a mutable captured local can hold when any of its closures runs.
    pub(crate) captured_decl: HashMap<(FileId, SemanticId), CacheEntry<LuaType>>,
    /// Syntax-only reachability for capture summaries, shared across locals in a file.
    pub(crate) captured_flow_reachability: HashMap<FileId, Rc<Vec<bool>>>,
    /// Memoized declaration/member traces; incoming path facts are part of the key.
    pub(crate) trace_state: HashMap<TraceStateKey, CacheEntry<Option<LuaType>>>,
    /// Fast negative/positive cache for `---@return_cast` lookup by call syntax.
    /// Most calls do not have `return_cast`, so caching the `None` result avoids
    /// repeatedly resolving callees and scanning signatures for the same call.
    pub(crate) return_cast:
        HashMap<(FileId, LuaSyntaxId), Option<(LuaExpr, LuaType, Option<LuaType>, bool)>>,
    pub(crate) callable_candidates: HashMap<(FileId, LuaSyntaxId), Vec<LuaFunctionType>>,
    pub(crate) local_callable_casts: HashMap<FileId, Rc<LocalCallableCasts>>,
    #[cfg(test)]
    pub(crate) local_callable_cast_scans: usize,
    pub(crate) call_site: HashMap<(FileId, LuaSyntaxId), CallSiteAnalysis>,
    pub(crate) call_site_signatures:
        HashMap<(FileId, LuaSyntaxId), Vec<(LuaFunctionType, TplBindings)>>,
    pub(crate) index_infos: HashMap<(FileId, LuaType), CacheEntry<Vec<IndexInfo>>>,
    pub(crate) inherited_dictionaries: HashMap<(FileId, LuaType), CacheEntry<Vec<LuaType>>>,
    pub(crate) member_infos: HashMap<(FileId, LuaType), Vec<MemberInfo>>,
    pub(crate) members_of_owner: HashMap<SemanticId, MemberList>,
    pub(crate) members_of_owner_named: HashMap<(SemanticId, SmolStr), MemberList>,
    pub(crate) member_info: HashMap<(FileId, LuaType, LuaMemberKey), Option<MemberInfo>>,
    pub(crate) type_check: HashMap<(FileId, LuaType, LuaType, bool), CacheEntry<bool>>,
    pub(crate) callable_functions: HashMap<(FileId, LuaType), Vec<LuaFunctionType>>,
    /// Active alias instances, shared with queries called during expansion.
    pub(crate) expanding_aliases: HashSet<LuaType>,
    pub(crate) alias_targets: HashMap<SemanticId, CacheEntry<Option<LuaType>>>,
    pub(crate) alias_heads: HashMap<(FileId, LuaType), CacheEntry<Option<LuaType>>>,
    pub(crate) type_defs: HashMap<LuaTypeDeclId, TypeDefCache>,
}

impl SemanticLocalCache {
    fn exhaust_evaluation(&mut self) {
        self.evaluation_exhausted = true;
        self.evaluation_errors
            .insert(self.evaluation_origin.clone());
        self.mark_incomplete();
    }

    fn discard_provisional(&mut self) {
        self.inherited_dictionaries
            .retain(|_, entry| entry.is_ready());
        self.member_receiver_targets
            .retain(|_, entry| entry.is_ready());
        self.member_callees.retain(|_, entry| entry.is_ready());
        self.member_call_effects.retain(|_, entry| entry.is_ready());
        self.expr_type.retain(|_, entry| entry.is_ready());
        self.decl_type.retain(|_, entry| entry.is_ready());
        self.index_infos.retain(|_, entry| entry.is_ready());
        self.member_type.retain(|_, entry| entry.is_ready());
        self.expr_type_at.retain(|_, entry| entry.is_ready());
        self.trace_state.retain(|_, entry| entry.is_ready());
        self.loop_carried.retain(|_, entry| entry.is_ready());
        self.captured_decl.retain(|_, entry| entry.is_ready());
        self.type_check.retain(|_, entry| entry.is_ready());
        self.alias_targets.retain(|_, entry| entry.is_ready());
        self.alias_heads.retain(|_, entry| entry.is_ready());
    }

    /// Count incomplete semantic queries; successful cache insertion does not advance this value.
    pub(crate) fn revision(&self) -> u64 {
        self.incomplete_revision.get()
    }

    pub(crate) fn mark_incomplete(&self) {
        self.incomplete_revision
            .set(self.incomplete_revision.get().wrapping_add(1));
    }

    /// No incomplete dependency has been observed since this snapshot.
    /// Successful cache insertions do not advance the revision; recursion and
    /// budget exhaustion do, so their partial results cannot prove absence.
    pub(crate) fn is_complete(&self, revision: u64) -> bool {
        self.revision() == revision
    }
}

#[cfg(test)]
mod inference_seed_regression_tests {
    use super::*;
    use emmylua_parser::{LuaAstNode, LuaBinaryExpr, LuaNameExpr};

    #[test]
    fn arithmetic_preserves_declared_return_slots_during_initializer_reentry() {
        for (field_type, expected) in [("integer", LuaType::Integer), ("number", LuaType::Number)] {
            for second_slot in [false, true] {
                for recursive_read in [false, true] {
                    let mut ws = crate::VirtualWorkspace::new();
                    let returns = if second_slot {
                        "integer, Counter"
                    } else {
                        "Counter"
                    };
                    let prefix = if second_slot { "0, " } else { "" };
                    let names = if second_slot { "ignored, item" } else { "item" };
                    let source = format!(
                        r#"---@class Counter
---@field count {field_type}
local Counter = {{}}
---@return {returns}
local function create() return {prefix}{{count = 1}} end
local {names} = create()
local value = 10 - item.count
"#
                    );
                    let file = ws.def(&source);
                    let model = ws.analysis.semantic_model(file);
                    let chunk = model.chunk().unwrap();
                    let read = chunk
                        .descendants::<LuaNameExpr>()
                        .find(|name| name.get_text() == "item")
                        .unwrap();
                    let decl = model.resolve_name(read.get_position()).unwrap();
                    let expression = chunk
                        .descendants::<LuaBinaryExpr>()
                        .next()
                        .unwrap()
                        .get_syntax_id();
                    let key = (file, decl);
                    if recursive_read {
                        // Hold the initializer open while a dependent arithmetic
                        // query reads a value with an explicit return contract.
                        model
                            .cache
                            .borrow_mut()
                            .decl_type
                            .insert(key.clone(), CacheEntry::InProgress);
                        assert_eq!(
                            model.type_of_expr(expression),
                            expected,
                            "pending {field_type}, second_slot={second_slot}"
                        );
                        assert!(
                            !model
                                .cache
                                .borrow()
                                .expr_type
                                .contains_key(&(file, expression))
                        );
                        model.cache.borrow_mut().decl_type.remove(&key);
                    }
                    for _ in 0..2 {
                        assert_eq!(
                            model.type_of_expr(expression),
                            expected,
                            "completed {field_type}, second_slot={second_slot}, recursive_read={recursive_read}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn other_recursive_declarations_keep_empty_seeds() {
        for (source, unknown_projection) in [
            ("local item = missing(); print(item)", true),
            (
                "---@return integer\nlocal function create() return 1 end\nlocal item = (create()); print(item)",
                false,
            ),
            ("local item = 1; print(item)", false),
            ("for _, item in pairs({}) do print(item) end", false),
            (
                "---@return integer\nlocal function create() return 1 end\nitem = create(); print(item)",
                false,
            ),
        ] {
            let mut ws = crate::VirtualWorkspace::new();
            let file = ws.def(source);
            let model = ws.analysis.semantic_model(file);
            let read = model
                .chunk()
                .unwrap()
                .descendants::<LuaNameExpr>()
                .filter(|name| name.get_text() == "item")
                .last()
                .unwrap();
            let decl = model.resolve_name(read.get_position()).unwrap();
            if unknown_projection {
                assert!(
                    model
                        .analysis()
                        .decl_type_lua(file, decl.clone())
                        .unwrap()
                        .is_unknown()
                );
            }
            model
                .cache
                .borrow_mut()
                .decl_type
                .insert((file, decl.clone()), CacheEntry::InProgress);
            let revision = model.cache.borrow().revision();
            assert_eq!(model.type_of_decl(&decl), None, "{source}");
            assert_eq!(model.cache.borrow().revision(), revision, "{source}");
        }
    }

    #[test]
    fn completed_value_replaces_provisional_arithmetic() {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def("---@return integer\nlocal function create() return 1 end\nlocal item = create()\nlocal value = 10 - item");
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        let read = chunk
            .descendants::<LuaNameExpr>()
            .find(|name| name.get_text() == "item")
            .unwrap();
        let decl = model.resolve_name(read.get_position()).unwrap();
        let expression = chunk
            .descendants::<LuaBinaryExpr>()
            .next()
            .unwrap()
            .get_syntax_id();
        let key = (file, decl);
        model
            .cache
            .borrow_mut()
            .decl_type
            .insert(key.clone(), CacheEntry::InProgress);
        assert_eq!(model.type_of_expr(expression), LuaType::Integer);
        assert!(
            !model
                .cache
                .borrow()
                .expr_type
                .contains_key(&(file, expression))
        );
        // Model completion with a broader result: the prior arithmetic must be
        // recomputed, while subsequent complete reads remain cached.
        model
            .cache
            .borrow_mut()
            .decl_type
            .insert(key, CacheEntry::Ready(Some(LuaType::Number)));
        let revision = model.cache.borrow().revision();
        for _ in 0..2 {
            assert_eq!(model.type_of_expr(expression), LuaType::Number);
        }
        assert_eq!(model.cache.borrow().revision(), revision);
    }

    #[test]
    fn captured_call_initializer_keeps_integer_arithmetic() {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(
            r#"---@class NumericLoad
---@field stacks integer
local NumericLoad = {}
---@param capacity integer
---@return NumericLoad
local function limit(capacity) return {stacks = 1} end
---@return integer
local function consume()
    local remaining = 100
    while remaining > 0 do
        local item = limit(remaining)
        remaining = remaining - item.stacks
        print(function() return item.stacks end)
    end
    return remaining
end
"#,
        );
        let diagnostics = ws
            .analysis
            .diagnose_file(file, tokio_util::sync::CancellationToken::new())
            .unwrap();
        assert!(
            !diagnostics.iter().any(|diagnostic| matches!(
                &diagnostic.code,
                Some(lsp_types::NumberOrString::String(code))
                    if code == "assign-type-mismatch" || code == "return-type-mismatch"
            )),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn recursive_call_seeds_preserve_available_projections() {
        for (annotation, expected) in [
            ("---@return any\n", LuaType::Any),
            (
                "---@return integer|unknown\n",
                LuaType::Union(
                    crate::LuaUnionType::from_vec(vec![LuaType::Integer, LuaType::Unknown]).into(),
                ),
            ),
            ("", LuaType::Integer),
        ] {
            let mut ws = crate::VirtualWorkspace::new();
            let file = ws.def(&format!("{annotation}local function create() return 1 end\nlocal item = create()\nprint(item)"));
            let model = ws.analysis.semantic_model(file);
            let read = model
                .chunk()
                .unwrap()
                .descendants::<LuaNameExpr>()
                .find(|name| name.get_text() == "item")
                .unwrap();
            let decl = model.resolve_name(read.get_position()).unwrap();
            // The shell supplies both annotated and locally inferred returns.
            // Preserve its available projection while initialization is pending.
            assert_eq!(
                model.analysis().decl_type_lua(file, decl.clone()),
                Some(expected.clone())
            );
            let key = (file, decl.clone());
            model
                .cache
                .borrow_mut()
                .decl_type
                .insert(key.clone(), CacheEntry::InProgress);
            let revision = model.cache.borrow().revision();
            assert_eq!(model.type_of_decl(&decl), Some(expected));
            assert!(!model.cache.borrow().is_complete(revision));
            assert!(matches!(
                model.cache.borrow().decl_type.get(&key),
                Some(CacheEntry::InProgress)
            ));
        }
    }
}
