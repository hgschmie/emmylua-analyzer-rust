//! Instantiate dependent function constraints without replacing inference sites.

use super::{
    unify::{self, TplBindings},
    vm::bind_signature_generics,
};
use crate::semantic_model::{SemanticModel, type_eval};
use crate::{GenericTpl, LuaFunctionType, LuaType, LuaTypeNode, TypeDefKind, VariadicType};

pub(crate) fn instantiate(
    model: &SemanticModel,
    ty: &LuaType,
    generics: &[GenericTpl],
    bindings: &TplBindings,
) -> LuaType {
    let ty = bind_signature_generics(ty, generics);
    let ty = unify::substitute(&ty, bindings);
    let ty = type_eval::expand_alias_generic(model, &ty);
    evaluate_constraint(model, &type_eval::eval_conditionals(model, &ty), 0)
}

fn evaluate_constraint(model: &SemanticModel, ty: &LuaType, depth: usize) -> LuaType {
    // Constraints may project recursively defined classes. Leave unresolved operators
    // intact rather than recursing indefinitely.
    if depth >= 32 {
        return ty.clone();
    }
    if let LuaType::Call(call) = ty {
        let operands: Vec<_> = call
            .get_operands()
            .iter()
            .map(|ty| evaluate_constraint(model, ty, depth + 1))
            .collect();
        let call = crate::LuaAliasCallType::new(call.get_call_kind(), operands);
        if call.get_call_kind() == crate::LuaAliasCallKind::Index {
            if let Some(value) = type_eval::expand_index_call(model, &call) {
                return evaluate_constraint(model, &value, depth + 1);
            }
        } else if call.get_call_kind() == crate::LuaAliasCallKind::KeyOf {
            if let Some(owner) = call.get_operands().first() {
                let keys = model
                    .member_infos(owner)
                    .into_iter()
                    .filter_map(|info| {
                        if let Some(name) = info.key.name() {
                            Some(LuaType::StringConst(smol_str::SmolStr::new(name).into()))
                        } else {
                            info.key.get_integer().map(LuaType::IntegerConst)
                        }
                    })
                    .collect::<Vec<_>>();
                if !keys.is_empty() {
                    return LuaType::from_vec(keys);
                }
            }
        }
        return LuaType::Call(std::rc::Rc::new(call));
    }
    ty.clone()
}

/// Defaults/constraints provide a temporary view for computed types such as
/// Parameters<F>. They must not pre-bind a later argument's inference variable.
fn fallback_bindings(
    model: &SemanticModel,
    generics: &[GenericTpl],
    bindings: &TplBindings,
) -> TplBindings {
    let mut result = bindings.clone();
    for _ in 0..generics.len() {
        let mut changed = false;
        for generic in generics {
            if result.contains_key(&generic.get_tpl_id()) {
                continue;
            }
            let Some(fallback) = generic
                .get_default_type()
                .or_else(|| generic.get_constraint())
            else {
                continue;
            };
            let value = instantiate(model, fallback, generics, &result);
            if !value.contains_tpl_node() {
                result.insert(generic.get_tpl_id(), value);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    result
}

fn computed_parameter(model: &SemanticModel, ty: &LuaType) -> bool {
    match ty {
        LuaType::Conditional(_) | LuaType::Call(_) => true,
        LuaType::Generic(generic) => model
            .type_def_of(&generic.get_base_type_id())
            .filter(|def| def.kind == TypeDefKind::Alias)
            .and_then(|def| model.alias_target(&def))
            .is_some_and(|target| matches!(target, LuaType::Conditional(_))),
        LuaType::Variadic(variadic) => match variadic.as_ref() {
            VariadicType::Base(base) => computed_parameter(model, base),
            VariadicType::Multi(types) => types.iter().any(|ty| computed_parameter(model, ty)),
        },
        _ => false,
    }
}

pub(crate) fn parameter_type(
    model: &SemanticModel,
    ty: &LuaType,
    fun: &LuaFunctionType,
    bindings: &TplBindings,
) -> LuaType {
    let generics = fun.get_generic_params();
    if generics.is_empty() {
        return unify::substitute(ty, bindings);
    }
    // Resolve each constraint against arguments already matched, while retaining
    // the template itself so the current argument can still infer its binding.
    let metadata: Vec<_> = generics
        .iter()
        .map(|generic| {
            GenericTpl::new(
                generic.get_tpl_id(),
                generic.get_param().name.clone(),
                generic
                    .get_constraint()
                    .map(|constraint| instantiate(model, constraint, generics, bindings)),
                generic.get_default_type().cloned(),
                generic.is_const(),
                generic.get_param().attributes.clone(),
            )
        })
        .collect();
    let ty = bind_signature_generics(ty, &metadata);
    if computed_parameter(model, &ty) {
        let overlay = fallback_bindings(model, generics, bindings);
        instantiate(model, &ty, &metadata, &overlay)
    } else {
        unify::substitute(&ty, bindings)
    }
}
