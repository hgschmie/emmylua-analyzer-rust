//! Visible writes through the default environment's named slots.

use crate::semantic_db::def::SemanticId;
use crate::semantic_model::SemanticModel;
use emmylua_parser::{LuaAstNode, LuaExpr, LuaIndexExpr, LuaIndexKey, LuaStat, LuaVarExpr};

/// Some(empty) is a real overwrite with no corresponding RHS expression.
/// Returned callables are possible values, not definite postconditions of calls.
pub(super) fn written_values(
    model: &SemanticModel,
    statement: &LuaStat,
    name: &str,
) -> Option<Vec<LuaExpr>> {
    let (targets, values) = match statement {
        LuaStat::AssignStat(assignment) => assignment.get_var_and_expr_list(),
        LuaStat::FuncStat(function) => (
            function.get_func_name().into_iter().collect(),
            function
                .get_closure()
                .map(LuaExpr::ClosureExpr)
                .into_iter()
                .collect(),
        ),
        _ => return None,
    };
    let mut matched = false;
    let mut replacements = Vec::new();
    for (slot, target) in targets.iter().enumerate() {
        let LuaVarExpr::IndexExpr(index) = target else {
            continue;
        };
        if !writes_name(model, index, name) {
            continue;
        }
        matched = true;
        if let Some(value) = values.get(slot) {
            replacements.push(value.clone());
        }
    }
    matched.then_some(replacements)
}

fn writes_name(model: &SemanticModel, index: &LuaIndexExpr, name: &str) -> bool {
    let Some(prefix) = index
        .get_prefix_expr()
        .and_then(super::super::guards::unparenthesized)
    else {
        return false;
    };
    let LuaExpr::NameExpr(environment) = prefix else {
        return false;
    };
    let is_default = match environment.get_name_text().as_deref() {
        Some("_G") => {
            model.file_facts().is_some_and(|facts| {
                facts
                    .find_visible_decl_before_offset("_G", environment.get_position())
                    .is_none()
            }) && matches!(model.resolve_name(environment.get_position()),
            Some(SemanticId::Decl(decl)) if model.is_std_file(decl.file_id))
        }
        Some("_ENV") => model.file_facts().is_some_and(|facts| {
            facts
                .find_visible_decl_before_offset("_ENV", environment.get_position())
                .is_none()
        }),
        _ => false,
    };
    if !is_default {
        return false;
    }
    match index.get_index_key() {
        Some(LuaIndexKey::Name(key)) => key.get_name_text() == name,
        Some(LuaIndexKey::String(key)) => key.get_value() == name,
        // A computed key can select the global name. Numeric keys cannot.
        Some(LuaIndexKey::Expr(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn local_global_table_shadow_is_not_an_environment_write() {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let file =
            ws.def("local _G={}; _G.type=function() return 'table' end; local observed=type(nil)");
        let model = ws.analysis.semantic_model(file);
        let chunk = model.chunk().unwrap();
        for statement in chunk.descendants::<LuaStat>() {
            assert!(
                written_values(&model, &statement, "type").is_none(),
                "{}",
                statement.get_text()
            );
        }
        // The effect scanner respects local scope independently of the global
        // declaration resolver's existing special treatment of _G member names.
    }
}
