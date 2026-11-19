//! Ref/Def type checks: nominal + inheritance (`is_sub_type_of` via semantic `super_names`).
//!
//! Parts still missing from semantic fall back: alias origin, enum field unions, call operator.

use crate::{LuaGenericType, LuaType, LuaTypeDeclId, TypeDefKind};

use super::context::TypeCheckContext;
use super::sub_type::{get_base_type_id, is_sub_type_of};
use super::{TypeCheckResult, check_general_type_compact, table_associated_type_def};

/// Generic inheritance matching: whether sub's inheritance chain contains a generic parent matching the target generic instance arguments.
fn is_sub_type_of_generic(
    context: &TypeCheckContext,
    sub: &LuaTypeDeclId,
    target: &LuaGenericType,
) -> bool {
    use std::collections::HashSet;
    let target_base = target.get_base_type_id();
    if sub == &target_base && target.get_params().is_empty() {
        return true;
    }
    let mut stack = vec![sub.clone()];
    let mut visited = HashSet::new();
    visited.insert(sub.clone());
    while let Some(current) = stack.pop() {
        for super_type in context.super_types_of(&current) {
            match super_type {
                LuaType::Generic(super_generic)
                    if super_generic.get_base_type_id() == target_base =>
                {
                    if super_generic.get_params() == target.get_params() {
                        return true;
                    }
                }
                LuaType::Ref(super_id) => {
                    if super_id == target_base && target.get_params().is_empty() {
                        return true;
                    }
                    if visited.insert(super_id.clone()) {
                        stack.push(super_id);
                    }
                }
                _ => {}
            }
        }
    }
    false
}

pub fn check_ref_type_compact(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    compact_type: &LuaType,
) -> TypeCheckResult {
    // The type definition must exist.
    let Some(type_def) = context.type_def_of(source_id) else {
        return Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type));
    };

    if type_def.kind == TypeDefKind::Enum {
        return check_ref_enum(context, source_id, compact_type);
    }

    check_ref_class(context, source_id, compact_type)
}

fn check_ref_enum(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    compact_type: &LuaType,
) -> TypeCheckResult {
    // Directly match the same type.
    if matches!(compact_type, LuaType::Def(id) | LuaType::Ref(id) if id == source_id) {
        return Ok(());
    }
    // Integer enum: broad Integer participates (semantic has no enum field union, so this is relaxed).
    if matches!(
        compact_type,
        LuaType::Integer | LuaType::IntegerConst(_) | LuaType::DocIntegerConst(_)
    ) {
        return Ok(());
    }
    // An enum's runtime table is still a table, so `pairs(DamageType)` etc. should be accepted.
    if matches!(compact_type, LuaType::Table) {
        return Ok(());
    }
    // Same base type name.
    if let Some(base_id) = get_base_type_id(compact_type)
        && is_sub_type_of(context, source_id, &base_id)
    {
        return Ok(());
    }
    Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
}

fn builtin_dictionary<'a>(
    context: &TypeCheckContext,
    generic: &'a LuaGenericType,
) -> Result<Option<(&'a LuaType, &'a LuaType)>, super::TypeCheckFailReason> {
    let [key, value] = generic.get_params().as_slice() else {
        return Ok(None);
    };
    let id = generic.get_base_type_id();
    if id != LuaTypeDeclId::global("table") {
        return Ok(None);
    }
    // Without library declarations the global table name still denotes the built-in dictionary.
    let builtin = context.require_complete(|| {
        context
            .type_def_of(&id)
            .is_none_or(|def| context.model.is_std_file(def.file_id))
    })?;
    Ok(builtin.then_some((key, value)))
}

fn check_record_dictionary(
    context: &TypeCheckContext,
    source_id: &LuaTypeDeclId,
    expected: &LuaType,
    key: &LuaType,
    value: &LuaType,
) -> TypeCheckResult {
    let source = LuaType::Ref(source_id.clone());
    // Primitive-backed classes can expose fields without supporting table iteration.
    if context.require_complete(|| {
        super::sub_type::is_sub_type_matching(context, source_id, |id| {
            matches!(id.get_id(), crate::LuaTypeIdentifier::Global(name)
                if matches!(name.as_str(), "userdata" | "function" | "string" | "number" | "integer" | "boolean" | "thread"))
        })
    })? {
        return Err(context.mismatch(&source, expected));
    }
    let fields = context
        .require_complete(|| crate::semantic_model::member::class_fields(context.model, &source))?;
    // Without a declared field or index signature an opaque class gives no evidence of a dictionary.
    if fields.get_fields().is_empty() && fields.get_index_access().is_empty() {
        return Err(context.mismatch(&source, expected));
    }
    // A broad string or number cannot establish membership in a literal key domain.
    let mut field_context = TypeCheckContext {
        strict_literals: true,
        ..context.clone()
    };
    // Lua removes a key when its value is nil, so optional fields can be absent.
    let value_or_nil = LuaType::from_vec(vec![value.clone(), LuaType::Nil]);
    let entries = fields
        .get_fields()
        .iter()
        .map(|(k, v)| (k.to_index_type(), v))
        .chain(
            fields
                .get_index_access()
                .iter()
                .map(|(k, v)| (Some(k.clone()), v)),
        );
    super::check_all(entries.map(|(actual_key, actual_value)| {
        let Some(actual_key) = actual_key else {
            return Err(field_context.mismatch(&source, expected));
        };
        let key_check = check_general_type_compact(&mut field_context, &actual_key, key);
        let value_check =
            check_general_type_compact(&mut field_context, actual_value, &value_or_nil);
        super::check_all([key_check, value_check])
    }))
}

fn check_ref_class(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    compact_type: &LuaType,
) -> TypeCheckResult {
    match compact_type {
        LuaType::Def(id) | LuaType::Ref(id) => {
            if source_id == id {
                return Ok(());
            }
            // Check subtype relationship (bidirectional: nominal assignment allows super ↔ sub).
            if is_sub_type_of(context, id, source_id) || is_sub_type_of(context, source_id, id) {
                return Ok(());
            }
            // Extra handling for enum targets (semantic has no field union, so fall back to base type name).
            if context.is_enum(id) {
                if let Some(base_id) = get_base_type_id(&LuaType::Ref(source_id.clone()))
                    && is_sub_type_of(context, id, &base_id)
                {
                    return Ok(());
                }
            }
            Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
        }
        LuaType::Array(array) => {
            let value = context
                .require_complete(|| context.model.infer_iter_from_type_def(source_id, 1))?
                .ok_or_else(|| context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))?;
            check_general_type_compact(context, &value, array.get_base())
        }
        LuaType::Table => Ok(()),
        LuaType::Generic(generic) => {
            if let Some((key, value)) = builtin_dictionary(context, generic)? {
                if context
                    .require_complete(|| is_sub_type_of_generic(context, source_id, generic))?
                {
                    return Ok(());
                }
                return check_record_dictionary(context, source_id, compact_type, key, value);
            }
            let base_type_id = generic.get_base_type_id();
            if (context.strict_subtype
                && context
                    .require_complete(|| is_sub_type_of_generic(context, source_id, generic))?)
                || (!context.strict_subtype
                    && (source_id == &base_type_id
                        || is_sub_type_of(context, &base_type_id, source_id)
                        || is_sub_type_of(context, source_id, &base_type_id)))
            {
                Ok(())
            } else {
                Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
            }
        }
        // Table/object/tuple/intersection structures: M0 uses base type name (`table`) plus nominal fallback (semantic member-structure checks are left for later).
        LuaType::TableConst(table) => {
            /*
               ---@class A
               local A = {}
               A.__index = A
            */
            if let Some(target_def) = table_associated_type_def(context.model, table)
                && target_def.kind == TypeDefKind::Class
            {
                check_general_type_compact(
                    context,
                    &LuaType::Ref(source_id.clone()),
                    &LuaType::Ref(target_def.type_decl_id()),
                )
            } else {
                if let Some(base_id) = get_base_type_id(compact_type)
                    && is_sub_type_of(context, source_id, &base_id)
                {
                    return Ok(());
                }
                Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
            }
        }
        LuaType::Object(_) | LuaType::Tuple(_) | LuaType::Intersection(_) => {
            if let Some(base_id) = get_base_type_id(compact_type)
                && is_sub_type_of(context, source_id, &base_id)
            {
                return Ok(());
            }
            Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
        }
        _ => {
            if let Some(base_type_id) = get_base_type_id(compact_type) {
                if source_id == &base_type_id
                    || is_sub_type_of(context, &base_type_id, source_id)
                    || is_sub_type_of(context, source_id, &base_type_id)
                {
                    Ok(())
                } else {
                    Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
                }
            } else {
                Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
            }
        }
    }
}
