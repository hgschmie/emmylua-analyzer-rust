//! Ref/Def type checks: nominal + inheritance (`is_sub_type_of` via semantic `super_names`).
//!
//! Parts still missing from semantic fall back: alias origin, enum field unions, call operator.

use crate::{InFiled, LuaGenericType, LuaType, LuaTypeDeclId, TypeDefKind};
use emmylua_parser::{LuaAst, LuaAstNode, LuaExpr, LuaTableExpr, LuaTableField};

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
            // A class-valued enum member keeps its own nominal identity, but is
            // also accepted wherever the containing enum is expected.
            if context.is_enum(id) && enum_contains_class(context, id, source_id) {
                return Ok(());
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

pub(super) fn enum_contains_class(
    context: &TypeCheckContext,
    enum_id: &LuaTypeDeclId,
    class_id: &LuaTypeDeclId,
) -> bool {
    let Some(def) = context.model.type_def_of(enum_id) else {
        return false;
    };
    let Some(class_def) = context.model.type_def_of(class_id) else {
        return false;
    };
    let model = context.model.model_for(def.file_id);
    let Some(tree) = model.syntax_tree() else {
        return false;
    };
    let Some(owner) = def
        .owner_syntax
        .and_then(|syntax| syntax.to_node_from_root(&tree.get_red_root()))
        .and_then(LuaAst::cast)
    else {
        return false;
    };
    let value = match owner {
        LuaAst::LuaLocalStat(stat) => stat.get_value_exprs().next(),
        LuaAst::LuaAssignStat(stat) => stat.get_var_and_expr_list().1.into_iter().next(),
        LuaAst::LuaTableField(field) => field.get_value_expr(),
        _ => None,
    };
    let Some(LuaExpr::TableExpr(table)) = value else {
        return false;
    };
    let Some(facts) = model.file_facts() else {
        return false;
    };
    let table_type = LuaType::TableConst(InFiled::new(def.file_id, table.get_range()));
    model.member_infos(&table_type).into_iter().any(|info| {
        // Inline class annotations belong to the field value, not to the
        // surrounding enum. Ordinary member projection can lose that identity.
        let annotated = info
            .id
            .as_ref()
            .and_then(|id| facts.member_by_id(id))
            .filter(|member| member.doc_type_syntax.is_none())
            .and_then(|member| member.value_syntax)
            .and_then(|syntax| syntax.to_node_from_root(&tree.get_red_root()))
            .and_then(LuaTableExpr::cast)
            .and_then(|value| value.get_parent::<LuaTableField>())
            .and_then(|field| facts.type_def_by_owner_syntax(field.get_syntax_id()))
            .filter(|def| def.kind == TypeDefKind::Class)
            .map(|def| model.type_def_ref(def));
        let member_type = annotated.unwrap_or_else(|| match &info.typ {
            LuaType::TableConst(table) => {
                super::super::member::table_const_class_type(&model, table).unwrap_or(info.typ)
            }
            _ => info.typ,
        });
        let (LuaType::Ref(member_id) | LuaType::Def(member_id)) = member_type else {
            return false;
        };
        // Enum-valued or unresolved members must not create recursive or
        // blanket compatibility with arbitrary classes.
        model.type_def_of(&member_id).is_some_and(|member_def| {
            member_def.kind == TypeDefKind::Class
                && super::def_extends_ctx(context.model, &class_def, &member_def, &mut Vec::new())
        })
    })
}
