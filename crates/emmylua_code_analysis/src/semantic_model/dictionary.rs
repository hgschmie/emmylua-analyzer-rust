//! Dictionary reads can miss a key; writes may delete it with nil.
//! Explicit fields retain their declared contract in either context.
use super::{SemanticModel, type_eval};
use crate::{LuaMemberKey, LuaType, TypeDefKind};
use emmylua_parser::{LuaAstNode, LuaIndexExpr, LuaIndexKey};

pub(crate) fn result(model: &SemanticModel, value: &LuaType, nullable: bool) -> LuaType {
    if nullable && model.strict_table_index() && !matches!(value, LuaType::Any | LuaType::Unknown) {
        LuaType::from_vec(vec![value.clone(), LuaType::Nil])
    } else {
        value.clone()
    }
}

#[derive(Clone, Copy)]
enum LookupMode {
    Read,
    Value,
    Write,
}

impl LookupMode {
    fn value(self, model: &SemanticModel, typ: &LuaType) -> LuaType {
        match self {
            Self::Read => result(model, typ, true),
            Self::Value => typ.clone(),
            Self::Write => LuaType::from_vec(vec![typ.clone(), LuaType::Nil]),
        }
    }
}

pub(crate) fn key_type(key: &LuaMemberKey) -> Option<LuaType> {
    match key {
        LuaMemberKey::Name(name) => Some(LuaType::StringConst(name.clone().into())),
        LuaMemberKey::Integer(n) => Some(LuaType::IntegerConst(*n)),
        _ => None,
    }
}

pub(crate) fn lookup(
    model: &SemanticModel,
    owner: &LuaType,
    key: &LuaType,
    nullable: bool,
) -> Option<LuaType> {
    let mode = if nullable {
        LookupMode::Read
    } else {
        LookupMode::Value
    };
    let normalized = normalized_key(key);
    lookup_inner(
        model,
        owner,
        normalized.as_ref().unwrap_or(key),
        mode,
        &mut Vec::new(),
    )
}
fn combine_values(values: Vec<LuaType>, mode: LookupMode) -> LuaType {
    if matches!(mode, LookupMode::Write) && values.len() > 1 {
        LuaType::Intersection(std::rc::Rc::new(crate::LuaIntersectionType::new(values)))
    } else {
        LuaType::from_vec(values)
    }
}

// Keep write constraints separate: the general compatibility checker currently
// treats target intersections as alternatives, which is unsuitable for a key
// that can select multiple required fields.
fn write_targets(typ: LuaType) -> Vec<LuaType> {
    match typ {
        LuaType::Intersection(intersection) => intersection
            .get_types()
            .iter()
            .cloned()
            .flat_map(write_targets)
            .collect(),
        typ => vec![typ],
    }
}

fn normalized_key(key: &LuaType) -> Option<LuaType> {
    match key {
        LuaType::FloatConst(value) => {
            crate::semantic_db::types::exact_integer(*value).map(LuaType::IntegerConst)
        }
        _ => None,
    }
}

fn is_standard_table(model: &SemanticModel, id: &crate::LuaTypeDeclId) -> bool {
    *id == crate::LuaTypeDeclId::global("table")
        && model
            .type_def_of(id)
            .is_none_or(|def| model.is_std_file(def.file_id))
}

fn lookup_inner(
    model: &SemanticModel,
    owner: &LuaType,
    key: &LuaType,
    mode: LookupMode,
    seen: &mut Vec<LuaType>,
) -> Option<LuaType> {
    if seen.len() >= 32 || seen.contains(owner) {
        return None;
    }
    seen.push(owner.clone());
    // Only ordinary dictionaries distribute key unions. Nominal classes and
    // explicit __index functions retain their own lookup contracts.
    let dictionary = matches!(owner, LuaType::TableGeneric(_) | LuaType::Object(_))
        || matches!(owner, LuaType::Generic(g) if is_standard_table(model, &g.get_base_type_id()));
    if dictionary {
        let value = lookup_keys(model, owner, key, mode, seen, &mut Vec::new());
        seen.pop();
        return value;
    }
    let value = lookup_atomic(model, owner, key, mode, seen);
    seen.pop();
    value
}

fn lookup_keys(
    model: &SemanticModel,
    owner: &LuaType,
    key: &LuaType,
    mode: LookupMode,
    owners: &mut Vec<LuaType>,
    keys: &mut Vec<LuaType>,
) -> Option<LuaType> {
    if keys.len() >= 32 || keys.contains(key) {
        return Some(LuaType::Unknown);
    }
    keys.push(key.clone());
    let homogeneous = match owner {
        LuaType::TableGeneric(params) if params.len() == 2 => Some((&params[0], &params[1])),
        LuaType::Generic(g)
            if is_standard_table(model, &g.get_base_type_id()) && g.get_params().len() == 2 =>
        {
            Some((&g.get_params()[0], &g.get_params()[1]))
        }
        LuaType::Object(object)
            if object.get_fields().is_empty() && object.get_index_access().len() == 1 =>
        {
            object.get_index_access().first().map(|(k, v)| (k, v))
        }
        _ => None,
    };
    let expanded = match key {
        LuaType::Ref(id) | LuaType::Def(id) => model
            .type_def_of(id)
            .filter(|def| def.kind == TypeDefKind::Alias)
            .and_then(|def| model.alias_target(&def)),
        LuaType::Generic(_) => {
            let expanded = type_eval::expand_alias_generic(model, key);
            (expanded != *key).then_some(expanded)
        }
        LuaType::MultiLineUnion(union) => Some(union.to_union()),
        _ => None,
    };
    let value = if *key == LuaType::Nil {
        Some(LuaType::Nil)
    } else if let LuaType::Union(union) = key {
        Some(combine_values(
            union
                .into_vec()
                .iter()
                .map(|key| {
                    lookup_keys(model, owner, key, mode, owners, keys).unwrap_or(LuaType::Nil)
                })
                .collect(),
            mode,
        ))
    } else if let Some(expanded) = expanded {
        // Keep an exact alias match as one signature check. Expanding thousands
        // of literal keys and rechecking each against that alias is quadratic.
        // Still inspect the alias for nil and bound recursively growing generics.
        if let Some((declared, value)) = homogeneous
            && (declared == key || matches!(declared, LuaType::Any | LuaType::Unknown))
        {
            match key_nil_status(model, &expanded, keys) {
                None => Some(LuaType::Unknown),
                Some(true) => Some(LuaType::from_vec(vec![
                    mode.value(model, value),
                    LuaType::Nil,
                ])),
                Some(false) => Some(mode.value(model, value)),
            }
        } else {
            lookup_keys(model, owner, &expanded, mode, owners, keys)
        }
    } else {
        lookup_atomic(model, owner, key, mode, owners)
    };
    keys.pop();
    value
}

fn key_nil_status(model: &SemanticModel, key: &LuaType, seen: &mut Vec<LuaType>) -> Option<bool> {
    if seen.len() >= 32 || seen.contains(key) {
        return None;
    }
    seen.push(key.clone());
    let result = match key {
        LuaType::Nil => Some(true),
        LuaType::Ref(id) | LuaType::Def(id) => match model
            .type_def_of(id)
            .filter(|def| def.kind == TypeDefKind::Alias)
            .and_then(|def| model.alias_target(&def))
        {
            Some(target) => key_nil_status(model, &target, seen),
            None => Some(false),
        },
        LuaType::Generic(_) => {
            let target = type_eval::expand_alias_generic(model, key);
            if target == *key {
                Some(false)
            } else {
                key_nil_status(model, &target, seen)
            }
        }
        LuaType::Union(union) => union.into_vec().iter().try_fold(false, |nullable, key| {
            key_nil_status(model, key, seen).map(|branch| nullable || branch)
        }),
        LuaType::MultiLineUnion(union) => key_nil_status(model, &union.to_union(), seen),
        _ => Some(false),
    };
    seen.pop();
    result
}

fn lookup_atomic(
    model: &SemanticModel,
    owner: &LuaType,
    key: &LuaType,
    mode: LookupMode,
    seen: &mut Vec<LuaType>,
) -> Option<LuaType> {
    match owner {
        LuaType::TableGeneric(params) if params.len() == 2 => {
            Some(signature(model, &params[0], &params[1], key, mode))
        }
        LuaType::Generic(generic)
            if is_standard_table(model, &generic.get_base_type_id())
                && generic.get_params().len() == 2 =>
        {
            let params = generic.get_params();
            Some(signature(model, &params[0], &params[1], key, mode))
        }
        LuaType::Object(object) => {
            if let Some(member_key) = type_eval::member_key_from_literal(key)
                && let Some(value) = object.get_fields().get(&member_key)
            {
                Some(value.clone())
            } else if object.get_index_access().is_empty() {
                None
            } else {
                let mut values: Vec<_> = object
                    .get_index_access()
                    .iter()
                    .filter(|(declared, _)| super::type_check::is_compatible(model, key, declared))
                    .map(|(_, value)| mode.value(model, value))
                    .collect();
                for (field, value) in object.get_fields() {
                    if let Some(field_key) = key_type(field)
                        && super::type_check::is_compatible(model, &field_key, key)
                        && !values.contains(value)
                    {
                        values.push(value.clone());
                    }
                }
                Some(if values.is_empty() {
                    LuaType::Nil
                } else {
                    combine_values(values, mode)
                })
            }
        }
        LuaType::Ref(id) | LuaType::Def(id) => model
            .type_def_of(id)
            .filter(|def| def.kind == TypeDefKind::Alias)
            .and_then(|def| model.alias_target(&def))
            .and_then(|target| lookup_inner(model, &target, key, mode, seen)),
        LuaType::Generic(_) => {
            let target = type_eval::expand_alias_generic(model, owner);
            if target == *owner {
                None
            } else {
                lookup_inner(model, &target, key, mode, seen)
            }
        }
        LuaType::Union(union) => {
            let values: Vec<_> = union
                .into_vec()
                .iter()
                .filter_map(|owner| lookup_inner(model, owner, key, mode, seen))
                .collect();
            (!values.is_empty()).then(|| combine_values(values, mode))
        }
        _ => None,
    }
}
fn signature(
    model: &SemanticModel,
    declared: &LuaType,
    value: &LuaType,
    key: &LuaType,
    mode: LookupMode,
) -> LuaType {
    let revision = model.cache.borrow().revision();
    let compatible = super::type_check::is_compatible(model, key, declared);
    if !model.cache.borrow().is_complete(revision) {
        // An unfinished alias query does not prove that the key is absent.
        LuaType::Unknown
    } else if compatible {
        mode.value(model, value)
    } else {
        LuaType::Nil
    }
}

impl SemanticModel<'_> {
    pub fn dictionary_index_type(&self, index: &LuaIndexExpr, nullable: bool) -> Option<LuaType> {
        let mode = if nullable {
            LookupMode::Read
        } else {
            LookupMode::Value
        };
        self.dictionary_index_type_with_mode(index, mode)
    }

    pub(crate) fn dictionary_index_write_types(
        &self,
        index: &LuaIndexExpr,
    ) -> Option<Vec<LuaType>> {
        self.dictionary_index_type_with_mode(index, LookupMode::Write)
            .map(write_targets)
    }

    fn dictionary_index_type_with_mode(
        &self,
        index: &LuaIndexExpr,
        mode: LookupMode,
    ) -> Option<LuaType> {
        let prefix = index.get_prefix_expr()?;
        let prefix = self.type_of_expr_at(prefix.get_syntax_id(), prefix.get_position());
        let key = match index.get_index_key()? {
            LuaIndexKey::Name(n) => {
                LuaType::StringConst(smol_str::SmolStr::new(n.get_name_text()).into())
            }
            LuaIndexKey::String(s) => {
                LuaType::StringConst(smol_str::SmolStr::new(s.get_value()).into())
            }
            LuaIndexKey::Integer(n) => match n.get_number_value() {
                emmylua_parser::NumberResult::Int(n) => LuaType::IntegerConst(n),
                emmylua_parser::NumberResult::Uint(n) => i64::try_from(n)
                    .map(LuaType::IntegerConst)
                    .unwrap_or(LuaType::Integer),
                emmylua_parser::NumberResult::Float(n) => {
                    crate::semantic_db::types::exact_integer(n)
                        .map(LuaType::IntegerConst)
                        .unwrap_or(LuaType::FloatConst(n))
                }
                _ => return None,
            },
            LuaIndexKey::Expr(expr) => {
                self.type_of_expr_at(expr.get_syntax_id(), expr.get_position())
            }
            _ => return None,
        };
        let normalized = normalized_key(&key);
        lookup_inner(
            self,
            &prefix,
            normalized.as_ref().unwrap_or(&key),
            mode,
            &mut Vec::new(),
        )
    }
}

#[cfg(test)]
#[test]
fn dictionary_named_table_requires_standard_identity() {
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("---@class table<K,V>\n---@field fixed string");
    let owner = ws.ty("table<integer,boolean>");
    let model = ws.analysis.semantic_model(file);
    assert_eq!(lookup(&model, &owner, &LuaType::Integer, false), None);
}

#[cfg(test)]
#[test]
fn dictionary_dynamic_object_keys_include_fields_and_protect_writes() {
    let mut ws = crate::VirtualWorkspace::new();
    let mut config = ws.get_emmyrc();
    config.strict.table_index = true;
    ws.update_emmyrc(config);
    let file = ws.def("");
    let owner = ws.ty("{required:string,[string]:number}");
    let model = ws.analysis.semantic_model(file);
    assert_eq!(
        lookup(&model, &owner, &LuaType::String, true),
        Some(LuaType::from_vec(vec![
            LuaType::String,
            LuaType::Number,
            LuaType::Nil
        ]))
    );
    let target = lookup_inner(
        &model,
        &owner,
        &LuaType::String,
        LookupMode::Write,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(
        !write_targets(target.clone())
            .iter()
            .all(|target| super::type_check::is_compatible(&model, &LuaType::Nil, target))
    );
    assert!(
        !write_targets(target)
            .iter()
            .all(|target| super::type_check::is_compatible(&model, &LuaType::String, target))
    );
    assert_eq!(
        lookup(
            &model,
            &owner,
            &LuaType::StringConst(smol_str::SmolStr::new("required").into()),
            true
        ),
        Some(LuaType::String)
    );
}
