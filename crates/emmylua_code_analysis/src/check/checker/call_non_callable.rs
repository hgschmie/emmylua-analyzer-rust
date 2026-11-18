//! # call_non_callable — calling a non-function type
//!
//! M0: callee type checked via `is_callable` (Function/DocFunction/Signature/Any/Unknown/
//! Nil pass; named types with `---@overload` / `---@operator call` pass; union passes only if all components are callable).

use emmylua_parser::{LuaAstNode, LuaCallExpr, LuaLocalStat};

use crate::DiagnosticCode;
use crate::semantic_db::def::TypeDefKind;
use crate::semantic_model::{SemanticModel, type_eval::expand_alias_generic};
use crate::{LuaType, LuaTypeDeclId};

use super::{CheckContext, Checker};
use crate::semantic_model::render::humanize_type;

pub struct CallNonCallableChecker;

impl Checker for CallNonCallableChecker {
    const CODES: &[DiagnosticCode] = &[DiagnosticCode::CallNonCallable];

    fn check(context: &mut CheckContext<'_>, semantic_model: &SemanticModel<'_>) {
        let Some(tree) = semantic_model.syntax_tree() else {
            return;
        };
        let root = tree.get_red_root();
        for call_expr in root.descendants().filter_map(LuaCallExpr::cast) {
            check_call(context, semantic_model, &call_expr);
        }
    }
}

fn check_call(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    call_expr: &LuaCallExpr,
) {
    let Some(prefix_expr) = call_expr.get_prefix_expr() else {
        return;
    };
    let callee_ty = callee_type(semantic_model, call_expr, &prefix_expr);
    if is_callable(semantic_model, &callee_ty) {
        return;
    }
    context.add_diagnostic(
        DiagnosticCode::CallNonCallable,
        prefix_expr.get_range(),
        t!(
            "Cannot call expression of type `%{name}`.",
            name = humanize_type(semantic_model, &callee_ty)
        ),
    );
}

fn callee_type(
    model: &SemanticModel,
    call: &LuaCallExpr,
    callee: &emmylua_parser::LuaExpr,
) -> LuaType {
    let emmylua_parser::LuaExpr::NameExpr(name) = callee else {
        return model.type_of_expr(callee.get_syntax_id());
    };
    if let Some(asserted) = crate::semantic_model::flow::inline_asserted_type(model, callee) {
        return asserted;
    }
    let position = name.get_position();
    let Some(decl) = model.resolve_name(position) else {
        return model.type_of_expr(callee.get_syntax_id());
    };
    // Initializers use the declared target contract. An ordinary call observes
    // the current value, including a function assigned since the declaration.
    if call
        .syntax()
        .ancestors()
        .any(|node| LuaLocalStat::cast(node).is_some())
    {
        model.type_of_decl_assign_target_at(&decl, position)
    } else {
        model.type_of_decl_at(&decl, position)
    }
}

fn is_callable(model: &SemanticModel, ty: &LuaType) -> bool {
    is_callable_inner(model, ty, &mut Vec::new())
}

fn is_callable_inner(
    model: &SemanticModel,
    ty: &LuaType,
    visited: &mut Vec<LuaTypeDeclId>,
) -> bool {
    match ty {
        LuaType::Function
        | LuaType::DocFunction(_)
        | LuaType::Signature(_)
        | LuaType::Any
        | LuaType::Unknown
        | LuaType::Nil
        | LuaType::SelfInfer
        | LuaType::Global => true,
        LuaType::TableConst(_) => !model.callable_functions_cached(ty).is_empty(),
        // Generic parameters: pass when the constraint is a callable type.
        LuaType::TplRef(tpl) => tpl
            .get_constraint()
            .is_some_and(|constraint| is_callable_inner(model, constraint, visited)),
        // Union: pass only if all non-nil components are callable (function|nil OK, function|integer reports).
        LuaType::Union(union) => union
            .into_vec()
            .iter()
            .all(|t| is_callable_inner(model, t, visited)),
        // Intersection: callable if any component is callable (`{ field: string } & fun()`).
        LuaType::Intersection(intersection) => intersection
            .get_types()
            .iter()
            .any(|t| is_callable_inner(model, t, visited)),
        // Match the class's call declarations used by call inference. A superclass's
        // constructor overload does not by itself make instances of a subclass callable.
        LuaType::Ref(id) | LuaType::Def(id) => is_named_callable(model, ty, id, visited),
        LuaType::Generic(generic) => {
            is_named_callable(model, ty, &generic.get_base_type_id(), visited)
        }
        _ => false,
    }
}

fn is_named_callable(
    model: &SemanticModel,
    ty: &LuaType,
    id: &LuaTypeDeclId,
    visited: &mut Vec<LuaTypeDeclId>,
) -> bool {
    if visited.contains(id) {
        return false;
    }
    // Unresolved types are handled by analyze_error, not call-non-callable.
    let Some(def) = model.type_def_of(id) else {
        return true;
    };
    if def.kind == TypeDefKind::Alias {
        let target = if matches!(ty, LuaType::Generic(_)) {
            Some(expand_alias_generic(model, ty))
        } else {
            model.alias_target(&def)
        };
        let Some(target) = target else {
            return false;
        };
        visited.push(id.clone());
        let callable = is_callable_inner(model, &target, visited);
        visited.pop();
        return callable;
    }
    !def.call_overloads.is_empty()
        || model
            .file_facts_of(def.file_id)
            .is_some_and(|facts| facts.operator_of(&def.id, "call").is_some())
}
