use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaDocType, LuaTableExpr, LuaVarExpr};

use crate::DiagnosticCode;
use crate::semantic_db::def::{TypeDef, TypeDefKind};
use crate::semantic_model::SemanticModel;
use crate::semantic_model::member::class_fields;
use crate::semantic_model::type_eval::{expand_alias_branches, expand_alias_head};
use crate::{LuaType, LuaTypeDeclId};

use super::{CheckContext, Checker, Mismatch};
use crate::semantic_model::render::humanize_type;
use crate::semantic_model::table_constructor::{TableConstructor, static_member_key, table_expr};
use crate::semantic_model::type_check::table_index_accepts;

pub struct CheckFieldChecker;

impl Checker for CheckFieldChecker {
    const CODES: &[DiagnosticCode] = &[DiagnosticCode::InjectField];

    fn check(context: &mut CheckContext<'_>, semantic_model: &SemanticModel<'_>) {
        let Some(tree) = semantic_model.syntax_tree() else {
            return;
        };
        for stat in tree
            .get_red_root()
            .descendants()
            .filter_map(LuaAssignStat::cast)
        {
            check_index_assign(context, semantic_model, &stat);
        }
    }
}

/// `test.a = 1` (test is a named type): if the target type has no such static member -> InjectField.
/// M0: only handle named classes without a parent type; dynamic keys, index signatures, and Object types are left for later.
fn check_index_assign(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    assign_stat: &LuaAssignStat,
) {
    let (vars, _) = assign_stat.get_var_and_expr_list();
    for var in vars {
        let LuaVarExpr::IndexExpr(index_expr) = var else {
            continue;
        };
        let Some(prefix) = index_expr.get_prefix_expr() else {
            continue;
        };
        let prefix_ty = semantic_model.type_of_expr(prefix.get_syntax_id());
        let Some(index_key) = index_expr.get_index_key() else {
            continue;
        };
        let Some(key) = static_member_key(semantic_model, &index_key) else {
            continue;
        };
        // Aliases must be expanded before deciding whether a field can be injected:
        // `---@alias Anything unknown` should allow `a.foo = 1`, while a class alias should
        // use the underlying class member surface.
        let prefix_ty = expand_alias_type(semantic_model, &prefix_ty).unwrap_or(prefix_ty);
        // `---@type { [number]: number }`: keys outside the object's index signature report injection errors.
        if let LuaType::Object(object) = &prefix_ty {
            if object.get_field(&key).is_none()
                && semantic_model
                    .applicable_index_infos(
                        &prefix_ty,
                        &key.to_index_type().unwrap_or(LuaType::Unknown),
                    )
                    .is_ok_and(|infos| infos.is_empty())
            {
                context.add_diagnostic(
                    DiagnosticCode::InjectField,
                    index_key
                        .get_range()
                        .unwrap_or_else(|| index_expr.get_range()),
                    t!(
                        "Fields cannot be injected into the reference of `%{class}` for `%{field}`. ",
                        class = humanize_type(semantic_model, &prefix_ty),
                        field = index_key.get_path_part()
                    ),
                );
            }
            continue;
        }
        let Some(def) = target_def(semantic_model, &prefix_ty) else {
            continue;
        };
        // Classes with a parent type or index signature (`[string]`) are handled broadly (M0).
        if !def.base_types.is_empty() || is_mapped_alias(semantic_model, &def) {
            continue;
        }
        if semantic_model.member_info(&prefix_ty, &key).is_some() {
            continue;
        }
        match semantic_model
            .applicable_index_infos(&prefix_ty, &key.to_index_type().unwrap_or(LuaType::Unknown))
        {
            Ok(infos) if infos.is_empty() => {}
            _ => continue,
        }
        context.add_diagnostic(
            DiagnosticCode::InjectField,
            index_key
                .get_range()
                .unwrap_or_else(|| index_expr.get_range()),
            t!(
                "Fields cannot be injected into the reference of `%{class}` for `%{field}`. ",
                class = humanize_type(semantic_model, &prefix_ty),
                field = index_key.get_path_part()
            ),
        );
    }
}

/// Resolve alias heads through the shared evaluator.
fn expand_alias_type(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> Option<LuaType> {
    let expanded = expand_alias_head(semantic_model, ty)?;
    (expanded != *ty).then_some(expanded)
}

/// Whether the alias target is a mapped type.
fn is_mapped_alias(semantic_model: &SemanticModel<'_>, def: &TypeDef) -> bool {
    let Some(syntax) = def.alias_type else {
        return false;
    };
    let Some(tree) = semantic_model.syntax_tree_of(def.file_id) else {
        return false;
    };
    let Some(node) = syntax.to_node_from_root(&tree.get_red_root()) else {
        return false;
    };
    matches!(LuaDocType::cast(node), Some(LuaDocType::Mapped(_)))
}

pub(super) fn table_literal_mismatches(
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
) -> Vec<Mismatch> {
    let mut errors = Vec::new();
    let mut pending = vec![TableCheck::Nested(table.clone(), target.clone())];
    while let Some(task) = pending.pop() {
        match task {
            TableCheck::Nested(table, target) => {
                if let Some(target) = expand_alias_branches(semantic_model, &target) {
                    check_table_head(&mut pending, &mut errors, semantic_model, &target, &table);
                }
            }
            TableCheck::Head(table, target) => {
                check_table_head(&mut pending, &mut errors, semantic_model, &target, &table)
            }
            TableCheck::NextAlternative {
                table,
                mut remaining,
                start,
                best,
            } => {
                let branch = errors.split_off(start);
                if branch.is_empty() {
                    continue;
                }
                let best = match best {
                    Some(previous) if previous.len() <= branch.len() => previous,
                    _ => branch,
                };
                if let Some(next) = remaining.next() {
                    pending.push(TableCheck::NextAlternative {
                        table: table.clone(),
                        remaining,
                        start,
                        best: Some(best),
                    });
                    pending.push(TableCheck::Head(table, next));
                } else {
                    errors.extend(best);
                }
            }
        }
    }
    errors
}

enum TableCheck {
    // Expand aliases once at a new constructor. A normalized union head
    // can still contain an unresolved alias cycle and must not expand again.
    Nested(LuaTableExpr, LuaType),
    Head(LuaTableExpr, LuaType),
    NextAlternative {
        table: LuaTableExpr,
        remaining: std::vec::IntoIter<LuaType>,
        start: usize,
        best: Option<Vec<Mismatch>>,
    },
}

fn check_table_head(
    pending: &mut Vec<TableCheck>,
    mismatches: &mut Vec<Mismatch>,
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
) {
    match target {
        LuaType::Array(array) => {
            for (field, key) in table.get_fields_with_keys() {
                if table_index_accepts(semantic_model, &LuaType::Integer, &key)
                    && let Some(value) = field.get_value_expr().and_then(|expr| table_expr(&expr))
                {
                    pending.push(TableCheck::Nested(value, array.get_base().clone()));
                }
            }
            return;
        }
        LuaType::Intersection(intersection) => {
            for component in intersection.get_types() {
                pending.push(TableCheck::Head(table.clone(), component.clone()));
            }
            return;
        }
        LuaType::Union(union) => {
            let mut alternatives =
                constructor_alternatives(semantic_model, union, table).into_iter();
            if let Some(first) = alternatives.next() {
                if alternatives.len() > 0 {
                    pending.push(TableCheck::NextAlternative {
                        table: table.clone(),
                        remaining: alternatives,
                        start: mismatches.len(),
                        best: None,
                    });
                }
                pending.push(TableCheck::Head(table.clone(), first));
            }
            return;
        }
        _ => {}
    }
    let class;
    let (shape, closed) = if let LuaType::Object(object) = target {
        (object.as_ref(), false)
    } else {
        let Some(def) = target_def(semantic_model, target) else {
            return;
        };
        if def.kind != TypeDefKind::Class {
            return;
        }
        // Only the standard-library dictionary bypasses closed-record checks.
        if let LuaType::Generic(generic) = target
            && generic.get_base_type_id() == LuaTypeDeclId::global("table")
            && semantic_model.is_std_file(def.file_id)
        {
            return;
        }
        class = class_fields(semantic_model, target);
        (&class, true)
    };
    let members = shape.get_fields();
    let constructor = TableConstructor::new(semantic_model, table);
    let mut missing = Vec::new();
    for (key, ty) in members {
        match constructor.member_value(key) {
            Ok(None) if !is_optional(semantic_model, ty) => missing.push(key.to_path()),
            Ok(Some(value)) => {
                if let Some(child) = value.expr.and_then(table_expr) {
                    pending.push(TableCheck::Nested(child, ty.clone()));
                }
            }
            _ => {}
        }
    }

    if closed {
        for (field, key) in table.get_fields_with_keys() {
            if static_member_key(semantic_model, &key).is_some_and(|key| members.contains_key(&key))
            {
                continue;
            }
            let expected = shape
                .get_index_access()
                .iter()
                .find(|(index, _)| table_index_accepts(semantic_model, index, &key))
                .map(|(_, value)| value);
            if let Some(expected) = expected {
                if let Some(value) = field.get_value_expr().and_then(|expr| table_expr(&expr)) {
                    pending.push(TableCheck::Nested(value, expected.clone()));
                }
            } else {
                mismatches.push(Mismatch {
                    code: DiagnosticCode::InjectField,
                    range: table.get_range(),
                    message: format!(
                        "Field `{}` cannot be injected into the target type.",
                        key.get_path_part()
                    ),
                });
            }
        }
    }

    missing.sort_unstable();
    report_missing_fields(mismatches, table, missing);
}

fn constructor_alternatives(
    model: &SemanticModel,
    union: &crate::LuaUnionType,
    table: &LuaTableExpr,
) -> Vec<LuaType> {
    // A constructor cannot match a primitive. A single viable shape needs no
    // full subtree comparison here; the value checker already performs it.
    let mut viable = union.iter().filter(|ty| {
        !matches!(
            ty,
            LuaType::Nil
                | LuaType::Boolean
                | LuaType::String
                | LuaType::Integer
                | LuaType::Number
                | LuaType::Function
        )
    });
    if let Some(only) = viable.next()
        && viable.next().is_none()
        && !target_def(model, only).is_some_and(|def| def.kind == TypeDefKind::Alias)
    {
        return vec![only.clone()];
    }
    let source = LuaType::TableConst(crate::InFiled::new(model.file_id(), table.get_range()));
    let compatible: Vec<_> = union
        .iter()
        .filter(|component| {
            !target_def(model, component).is_some_and(|def| def.kind == TypeDefKind::Alias)
                && crate::semantic_model::type_check::check_type_at(
                    model,
                    &source,
                    component,
                    table.get_range(),
                    false,
                )
                .is_ok()
        })
        .cloned()
        .collect();
    if !compatible.is_empty() {
        return compatible;
    }

    // A value mismatch inside an otherwise matching record belongs to the
    // assignment or argument checker. Prefer that record to unrelated missing fields.
    let constructor = TableConstructor::new(model, table);
    let mut closest = Vec::new();
    let mut fewest_missing = usize::MAX;
    for component in union.iter() {
        let class;
        let shape = match component {
            LuaType::Object(object) => object.as_ref(),
            _ if target_def(model, component).is_some_and(|def| def.kind == TypeDefKind::Class) => {
                class = class_fields(model, component);
                &class
            }
            _ => continue,
        };
        let missing = shape
            .get_fields()
            .iter()
            .filter(|(key, ty)| {
                matches!(constructor.member_value(key), Ok(None)) && !is_optional(model, ty)
            })
            .count();
        if missing < fewest_missing {
            closest.clear();
            fewest_missing = missing;
        }
        if missing == fewest_missing {
            closest.push(component.clone());
        }
    }
    closest
}

fn report_missing_fields(
    mismatches: &mut Vec<Mismatch>,
    table: &LuaTableExpr,
    missing: Vec<String>,
) {
    if !missing.is_empty() {
        mismatches.push(Mismatch {
            code: DiagnosticCode::MissingFields,
            range: table.get_range(),
            message: format!(
                "Missing required fields: {}",
                missing
                    .iter()
                    .map(|name| format!("`{}`", name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
}

/// The definition of a named target, before alias expansion.
fn target_def(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> Option<TypeDef> {
    match ty {
        LuaType::Ref(id) | LuaType::Def(id) => semantic_model.type_def_of(id),
        LuaType::Generic(generic) => semantic_model.type_def_of(&generic.get_base_type_id()),
        _ => None,
    }
}

/// Whether the field is optional (nullable / unknown).
fn is_optional(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> bool {
    fn contains_nil(ty: &LuaType) -> bool {
        match ty {
            LuaType::Any | LuaType::Unknown | LuaType::Nil => true,
            LuaType::Union(union) => union.iter().any(contains_nil),
            LuaType::Intersection(intersection) => {
                intersection.get_types().iter().all(contains_nil)
            }
            _ => false,
        }
    }

    let ty = expand_alias_branches(semantic_model, ty).unwrap_or_else(|| ty.clone());
    contains_nil(&ty)
}
