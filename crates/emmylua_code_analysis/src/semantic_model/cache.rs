//! Request-local cache shared across `SemanticModel` file views.
//!
//! A root model and its `model_for` views share one cache. Recursion state is
//! represented by `CacheEntry::InProgress` so re-entry checks stay O(1).

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

use super::infer::unify::TplBindings;
use super::member::MemberInfo;
use super::{CallSiteAnalysis, ResolvedMember};
use crate::IndexInfo;

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
    pub(super) fn new(cache: &'a super::LazyCache) -> Option<Self> {
        let query = QueryCacheScope::new(cache);
        let mut state = cache.borrow_mut();
        const MAX_EVALUATION_DEPTH: usize = 64;
        if state.evaluation_depth >= MAX_EVALUATION_DEPTH || state.evaluation_exhausted {
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
        const MAX_TYPE_VISITS: usize = 1_000_000;
        while let Some(ty) = pending.pop() {
            if cache.evaluation_exhausted || cache.evaluation_work >= MAX_TYPE_VISITS {
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

/// Memo key for one flow-trace state on a loop-free graph: `(file, subject, flow node)`.
///
/// The incoming path is deliberately absent. It is safe to omit only for flow
/// graphs without loops, where every route to a flow node crosses the same
/// guards, so the path is a function of the node. [`super::flow`] consults this
/// memo only in that case and recomputes otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TraceStateKey {
    pub(crate) file_id: FileId,
    pub(crate) subject: TraceSubject,
    pub(crate) flow_id: u32,
}

#[derive(Default)]
pub(crate) struct TypeDefCache {
    /// None 表示尚未计算, Some 空列表表示已知没有父类型.
    pub(crate) base_types: Option<Vec<LuaType>>,
}

#[derive(Default)]
pub(crate) struct SemanticLocalCache {
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
    pub(crate) expr_type: HashMap<(FileId, LuaSyntaxId), CacheEntry<LuaType>>,
    pub(crate) decl_type: HashMap<(FileId, SemanticId), CacheEntry<Option<LuaType>>>,
    pub(crate) member_type: HashMap<(FileId, SemanticId), CacheEntry<Option<LuaType>>>,
    pub(crate) resolve_member: HashMap<(FileId, LuaSyntaxId), Option<ResolvedMember>>,
    pub(crate) expr_type_at: HashMap<(FileId, LuaSyntaxId, TextSize), CacheEntry<LuaType>>,
    /// Cast-bearing flow bindings on the node and its ancestors, nearest first.
    pub(crate) expr_cast_flows: HashMap<(FileId, LuaSyntaxId), Rc<Vec<FlowId>>>,
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
    /// Memoized declaration/member-trace states for loop-free flow graphs. See
    /// [`TraceStateKey`] for why the path may be omitted from the key.
    pub(crate) trace_state: HashMap<TraceStateKey, CacheEntry<Option<LuaType>>>,
    /// Fast negative/positive cache for `---@return_cast` lookup by call syntax.
    /// Most calls do not have `return_cast`, so caching the `None` result avoids
    /// repeatedly resolving callees and scanning signatures for the same call.
    pub(crate) return_cast:
        HashMap<(FileId, LuaSyntaxId), Option<(LuaExpr, LuaType, Option<LuaType>, bool)>>,
    pub(crate) callable_candidates: HashMap<(FileId, LuaSyntaxId), Vec<LuaFunctionType>>,
    pub(crate) call_site: HashMap<(FileId, LuaSyntaxId), CallSiteAnalysis>,
    pub(crate) call_site_signatures:
        HashMap<(FileId, LuaSyntaxId), Vec<(LuaFunctionType, TplBindings)>>,
    pub(crate) index_infos: HashMap<(FileId, LuaType), CacheEntry<Vec<IndexInfo>>>,
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
        self.expr_type.retain(|_, entry| entry.is_ready());
        self.decl_type.retain(|_, entry| entry.is_ready());
        self.index_infos.retain(|_, entry| entry.is_ready());
        self.member_type.retain(|_, entry| entry.is_ready());
        self.expr_type_at.retain(|_, entry| entry.is_ready());
        self.trace_state.retain(|_, entry| entry.is_ready());
        self.type_check.retain(|_, entry| entry.is_ready());
        self.alias_targets.retain(|_, entry| entry.is_ready());
        self.alias_heads.retain(|_, entry| entry.is_ready());
    }

    pub(crate) fn revision(&self) -> u64 {
        self.incomplete_revision.get()
    }

    pub(crate) fn mark_incomplete(&self) {
        self.incomplete_revision
            .set(self.incomplete_revision.get().wrapping_add(1));
    }

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
}
