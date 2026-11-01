//! Reaching global writes can replace names still resolved to stdlib.

use super::reaching_values::reaching_values;
use crate::semantic_db::{
    def::{DeclKind, SemanticId},
    flow::LuaClosureId,
};
use crate::semantic_model::SemanticModel;
use emmylua_parser::{LuaAstNode, LuaExpr, LuaNameExpr, LuaStat, LuaVarExpr};
use std::rc::Rc;

pub(crate) struct GlobalNameValues {
    pub(crate) may_keep_builtin: bool,
    pub(crate) values: Vec<LuaExpr>,
}

/// Keep the builtin possible unless a single definition reaches every path.
/// Possible replacements also expose their visible bodies to call-effect scanning.
pub(crate) fn global_name_values(model: &SemanticModel, name: &LuaNameExpr) -> GlobalNameValues {
    let mut result = GlobalNameValues {
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
        .member_global_name_writes
        .get(&key)
        .cloned();
    let definitions = cached.unwrap_or_else(|| {
        let syntax = Rc::new(
            chunk
                .descendants::<LuaStat>()
                .filter(|statement| written_name(statement, &text).is_some())
                .map(|statement| statement.get_syntax_id())
                .collect::<Vec<_>>(),
        );
        model
            .cache
            .borrow_mut()
            .member_global_name_writes
            .insert(key, syntax.clone());
        syntax
    });
    let query_closure = LuaClosureId::from_node(name.syntax());
    for syntax in definitions.iter() {
        let Some(statement) = syntax
            .to_node_from_root(chunk.syntax())
            .and_then(LuaStat::cast)
        else {
            continue;
        };
        let Some(target) = written_name(&statement, &text) else {
            continue;
        };
        let id = SemanticId::decl(model.file_id(), target.get_range());
        if !facts
            .decl_by_id(&id)
            .is_some_and(|decl| matches!(decl.kind, DeclKind::Global))
        {
            continue;
        }
        let definition_closure = LuaClosureId::from_node(statement.syntax());
        if definition_closure == query_closure {
            let values = reaching_values(model, &id, &LuaExpr::NameExpr(name.clone()));
            if !values.is_empty() && values.iter().all(Option::is_some) {
                result.may_keep_builtin = false;
            }
            result.values.extend(values.into_iter().flatten());
        } else if definition_closure
            .get_range()
            .contains_range(query_closure.get_range())
            && statement.get_range().end() <= name.get_position()
        {
            // An enclosing definition can be visible at invocation time, but
            // this closure's CFG cannot prove which outer path invoked it.
            result.values.extend(
                facts
                    .decl_by_id(&id)
                    .and_then(|decl| decl.value_expr_syntax)
                    .and_then(|syntax| syntax.to_node_from_root(chunk.syntax()))
                    .and_then(LuaExpr::cast),
            );
        }
    }
    result
}

fn written_name(statement: &LuaStat, name: &str) -> Option<LuaNameExpr> {
    let vars = match statement {
        LuaStat::AssignStat(assignment) => assignment.get_var_and_expr_list().0,
        LuaStat::FuncStat(function) => function.get_func_name().into_iter().collect(),
        _ => return None,
    };
    vars.into_iter().find_map(|var| match var {
        LuaVarExpr::NameExpr(target) if target.get_name_text().as_deref() == Some(name) => {
            Some(target)
        }
        _ => None,
    })
}
