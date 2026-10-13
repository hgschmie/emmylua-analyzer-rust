//! Evaluate constructor fields and values without native recursion between union queries.

use std::rc::Rc;

use emmylua_parser::{LuaAstNode, LuaExpr, LuaTableExpr};
use rowan::TextRange;

use crate::semantic_model::SemanticModel;
use crate::semantic_model::cache::ConstructorQuery;
use crate::{DiagnosticCode, LuaMemberKey, LuaType, LuaUnionType};

use super::ConstructorDiagnostic;
use super::assign_type_mismatch::{
    TypeMismatch, entry_value_tasks, table_value_tasks, unresolved_constructor_mismatch,
    value_tasks,
};
use super::check_field::{field_tasks, static_member_key, union_alternatives};

#[derive(Clone, Copy)]
pub(super) enum UnionMode {
    All,
    Fields,
    Values,
}

pub(super) enum Task<'a> {
    Fields(LuaType, LuaTableExpr),
    Value(LuaType, LuaExpr),
    EntryValue(LuaType, LuaExpr),
    TableValues(LuaType, LuaTableExpr),
    Emit(ConstructorDiagnostic),
    Union(Rc<LuaUnionType>, LuaTableExpr, UnionMode),
    NextAlternative(Box<UnionState<'a>>),
    FinishAlternative(Box<UnionState<'a>>),
}

impl Task<'_> {
    pub(super) fn diagnostic(code: DiagnosticCode, range: TextRange, message: String) -> Self {
        Self::Emit(ConstructorDiagnostic {
            code,
            range,
            message,
        })
    }

    pub(super) fn mismatch(mismatch: TypeMismatch) -> Self {
        Self::diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            mismatch.range,
            mismatch.message,
        )
    }
}

pub(super) struct UnionState<'a> {
    query: ConstructorQuery<'a>,
    target: Rc<LuaUnionType>,
    table: LuaTableExpr,
    mode: UnionMode,
    alternatives: std::vec::IntoIter<LuaType>,
    best: Option<Vec<ConstructorDiagnostic>>,
    rejected_array: Option<Vec<ConstructorDiagnostic>>,
    named_key: Option<String>,
}

// Vec normally drops its elements in insertion order. Continuations own nested
// query scopes, so unwind them in the same LIFO order as normal evaluation.
struct Pending<'a>(Vec<Task<'a>>);

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        while let Some(task) = self.0.pop() {
            drop(task);
        }
    }
}

#[derive(Default)]
struct Outputs {
    root: Vec<ConstructorDiagnostic>,
    candidates: Vec<Vec<ConstructorDiagnostic>>,
}

impl Outputs {
    fn current(&mut self) -> &mut Vec<ConstructorDiagnostic> {
        self.candidates.last_mut().unwrap_or(&mut self.root)
    }

    fn finish_candidate(&mut self) -> Vec<ConstructorDiagnostic> {
        self.candidates
            .pop()
            .expect("a candidate output precedes its continuation")
    }
}

/// Each candidate completes both passes before selection; nested unions suspend
/// the parent in heap continuations. Active cache guards remain alive until the
/// corresponding continuation finishes, including during unwinding.
pub(super) fn evaluate(
    model: &SemanticModel<'_>,
    initial: Task<'_>,
) -> Option<Vec<ConstructorDiagnostic>> {
    let mut pending = Pending(vec![initial]);
    let mut outputs = Outputs::default();
    let mut unresolved_root = false;
    while let Some(task) = pending.0.pop() {
        let mut children = Vec::new();
        match task {
            Task::Fields(target, table) => field_tasks(&mut children, model, &target, &table),
            Task::Value(target, value) => value_tasks(&mut children, model, &target, &value),
            Task::EntryValue(target, value) => {
                entry_value_tasks(&mut children, model, &value, &target)
            }
            Task::TableValues(target, table) => {
                table_value_tasks(&mut children, model, &target, &table)
            }
            Task::Emit(diagnostic) => outputs.current().push(diagnostic),
            Task::Union(target, table, mode) => {
                match model.begin_constructor_diagnostics(
                    table.get_syntax_id(),
                    LuaType::Union(Rc::clone(&target)),
                ) {
                    Err(result) => deliver(
                        model,
                        &mut outputs,
                        &mut unresolved_root,
                        &target,
                        &table,
                        mode,
                        result,
                    ),
                    Ok(query) => {
                        let named_key = table.get_fields_with_keys().iter().find_map(|(_, key)| {
                            match static_member_key(model, key) {
                                Some(LuaMemberKey::Name(name)) => Some(name.to_string()),
                                _ => None,
                            }
                        });
                        let alternatives = union_alternatives(model, &target).into_iter();
                        children.push(Task::NextAlternative(Box::new(UnionState {
                            query,
                            target,
                            table,
                            mode,
                            alternatives,
                            best: None,
                            rejected_array: None,
                            named_key,
                        })));
                    }
                }
            }
            Task::NextAlternative(mut state) => {
                let mut selected = None;
                for alternative in state.alternatives.by_ref() {
                    // Named keys exclude an array alternative, but a record's
                    // own failure takes precedence over this fallback.
                    if matches!(alternative, LuaType::Array(_))
                        && let Some(name) = &state.named_key
                    {
                        state.rejected_array.get_or_insert_with(|| {
                            vec![ConstructorDiagnostic {
                                code: DiagnosticCode::InjectField,
                                range: state.table.get_range(),
                                message: format!(
                                    "Field `{name}` cannot be injected into an array type."
                                ),
                            }]
                        });
                        continue;
                    }
                    selected = Some(alternative);
                    break;
                }
                if let Some(alternative) = selected {
                    outputs.candidates.push(Vec::new());
                    children.push(Task::Fields(alternative.clone(), state.table.clone()));
                    children.push(Task::Value(
                        alternative,
                        LuaExpr::TableExpr(state.table.clone()),
                    ));
                    children.push(Task::FinishAlternative(state));
                } else {
                    let result = state.query.finish(state.best.or(state.rejected_array));
                    deliver(
                        model,
                        &mut outputs,
                        &mut unresolved_root,
                        &state.target,
                        &state.table,
                        state.mode,
                        result,
                    );
                }
            }
            Task::FinishAlternative(mut state) => {
                let diagnostics = outputs.finish_candidate();
                if diagnostics.is_empty() {
                    let result = state.query.finish(Some(diagnostics));
                    deliver(
                        model,
                        &mut outputs,
                        &mut unresolved_root,
                        &state.target,
                        &state.table,
                        state.mode,
                        result,
                    );
                } else {
                    // Failure count is a heuristic, independent of configured severity.
                    // Strict comparison preserves declaration-order ties.
                    if state
                        .best
                        .as_ref()
                        .is_none_or(|best| diagnostics.len() < best.len())
                    {
                        state.best = Some(diagnostics);
                    }
                    children.push(Task::NextAlternative(state));
                }
            }
        }
        // Producers append in evaluation order; the stack executes that same order.
        pending.0.extend(children.into_iter().rev());
    }
    if unresolved_root {
        None
    } else {
        Some(outputs.root)
    }
}

fn deliver(
    model: &SemanticModel<'_>,
    outputs: &mut Outputs,
    unresolved_root: &mut bool,
    target: &Rc<LuaUnionType>,
    table: &LuaTableExpr,
    mode: UnionMode,
    result: Option<Vec<ConstructorDiagnostic>>,
) {
    let output = outputs.current();
    match result {
        Some(diagnostics) => {
            output.extend(diagnostics.into_iter().filter(|diagnostic| match mode {
                UnionMode::All => true,
                UnionMode::Fields => matches!(
                    diagnostic.code,
                    DiagnosticCode::MissingFields | DiagnosticCode::InjectField
                ),
                UnionMode::Values => diagnostic.code == DiagnosticCode::AssignTypeMismatch,
            }))
        }
        None => match mode {
            UnionMode::All => *unresolved_root = true,
            UnionMode::Fields => {}
            UnionMode::Values => {
                let mismatch = unresolved_constructor_mismatch(
                    model,
                    &LuaType::Union(Rc::clone(target)),
                    table,
                );
                output.push(ConstructorDiagnostic {
                    code: DiagnosticCode::AssignTypeMismatch,
                    range: mismatch.range,
                    message: mismatch.message,
                });
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LuaUnionMembers, VirtualWorkspace};

    #[test]
    fn suspended_constructor_queries_clean_up_on_unwind() {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def("local value = {}");
        let table = ws.get_node::<LuaTableExpr>(file);
        let model = ws.analysis.semantic_model(file);
        let target = Rc::new(LuaUnionType::Multi(LuaUnionMembers::new(vec![
            LuaType::String,
            LuaType::Integer,
        ])));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut pending = Pending(Vec::new());
            for key in [LuaType::String, LuaType::Integer] {
                let query = model
                    .begin_constructor_diagnostics(table.get_syntax_id(), key)
                    .ok()
                    .unwrap();
                pending.0.push(Task::FinishAlternative(Box::new(UnionState {
                    query,
                    target: Rc::clone(&target),
                    table: table.clone(),
                    mode: UnionMode::All,
                    alternatives: Vec::new().into_iter(),
                    best: None,
                    rejected_array: None,
                    named_key: None,
                })));
            }
            panic!("failure with a suspended parent and child");
        }));
        assert!(result.is_err());
        for key in [LuaType::String, LuaType::Integer] {
            let retry = model
                .begin_constructor_diagnostics(table.get_syntax_id(), key.clone())
                .expect("abandoned query must be retried");
            assert!(retry.finish(Some(Vec::new())).unwrap().is_empty());
            assert!(
                matches!(model.begin_constructor_diagnostics(table.get_syntax_id(), key), Err(Some(diagnostics)) if diagnostics.is_empty())
            );
        }
    }
}
