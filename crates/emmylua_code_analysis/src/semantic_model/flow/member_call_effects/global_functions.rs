//! Reaching named global functions can replace a name still resolved to stdlib.

use super::reaching_values::reaching_values;
use crate::semantic_db::{
    def::{DeclKind, SemanticId},
    flow::LuaClosureId,
};
use crate::semantic_model::SemanticModel;
use emmylua_parser::{LuaAstNode, LuaExpr, LuaFuncStat, LuaNameExpr, LuaVarExpr};
use std::rc::Rc;

pub(crate) struct GlobalFunctionValues {
    pub(crate) may_keep_builtin: bool,
    pub(crate) values: Vec<LuaExpr>,
}

/// Keep the builtin possible unless a single definition reaches every path.
/// Possible replacements also expose their visible bodies to call-effect scanning.
pub(crate) fn global_function_values(
    model: &SemanticModel,
    name: &LuaNameExpr,
) -> GlobalFunctionValues {
    let mut result = GlobalFunctionValues {
        may_keep_builtin: true,
        values: Vec::new(),
    };
    let (Some(text), Some(chunk), Some(facts)) =
        (name.get_name_text(), model.chunk(), model.file_facts())
    else {
        return result;
    };
    let key = (model.file_id(), text.as_str().into());
    let cached = model
        .cache
        .borrow()
        .member_global_functions
        .get(&key)
        .cloned();
    let definitions = cached.unwrap_or_else(|| {
        let syntax = Rc::new(
            chunk
                .descendants::<LuaFuncStat>()
                .filter_map(|function| {
                    let Some(LuaVarExpr::NameExpr(target)) = function.get_func_name() else {
                        return None;
                    };
                    (target.get_name_text().as_ref() == Some(&text))
                        .then(|| function.get_syntax_id())
                })
                .collect::<Vec<_>>(),
        );
        model
            .cache
            .borrow_mut()
            .member_global_functions
            .insert(key, syntax.clone());
        syntax
    });
    let query_closure = LuaClosureId::from_node(name.syntax());
    for syntax in definitions.iter() {
        let Some(function) = syntax
            .to_node_from_root(chunk.syntax())
            .and_then(LuaFuncStat::cast)
        else {
            continue;
        };
        let Some(LuaVarExpr::NameExpr(target)) = function.get_func_name() else {
            continue;
        };
        let id = SemanticId::decl(model.file_id(), target.get_range());
        if !facts
            .decl_by_id(&id)
            .is_some_and(|decl| matches!(decl.kind, DeclKind::Global))
        {
            continue;
        }
        let definition_closure = LuaClosureId::from_node(function.syntax());
        if definition_closure == query_closure {
            let values = reaching_values(model, &id, &LuaExpr::NameExpr(name.clone()));
            if !values.is_empty() && values.iter().all(Option::is_some) {
                result.may_keep_builtin = false;
            }
            result.values.extend(values.into_iter().flatten());
        } else if definition_closure
            .get_range()
            .contains_range(query_closure.get_range())
            && function.get_range().end() <= name.get_position()
        {
            // An enclosing definition can be visible at invocation time, but
            // this closure's CFG cannot prove which outer path invoked it.
            result
                .values
                .extend(function.get_closure().map(LuaExpr::ClosureExpr));
        }
    }
    result
}
