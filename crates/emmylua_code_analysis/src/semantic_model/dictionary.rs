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
    lookup_inner(model, owner, key, mode, &mut Vec::new())
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
        || matches!(owner, LuaType::Generic(g) if g.get_base_type_id().get_name() == "table");
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
            if g.get_base_type_id().get_name() == "table" && g.get_params().len() == 2 =>
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
        Some(LuaType::from_vec(
            union
                .into_vec()
                .iter()
                .map(|key| {
                    lookup_keys(model, owner, key, mode, owners, keys).unwrap_or(LuaType::Nil)
                })
                .collect(),
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
            if generic.get_base_type_id().get_name() == "table"
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
                let values: Vec<_> = object
                    .get_index_access()
                    .iter()
                    .filter(|(declared, _)| {
                        super::type_check::is_assign_compatible(model, key, declared)
                    })
                    .map(|(_, value)| mode.value(model, value))
                    .collect();
                Some(if values.is_empty() {
                    LuaType::Nil
                } else {
                    LuaType::from_vec(values)
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
            (!values.is_empty()).then(|| LuaType::from_vec(values))
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
    if super::type_check::is_assign_compatible(model, key, declared) {
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

    pub(crate) fn dictionary_index_write_type(&self, index: &LuaIndexExpr) -> Option<LuaType> {
        self.dictionary_index_type_with_mode(index, LookupMode::Write)
    }

    fn dictionary_index_type_with_mode(
        &self,
        index: &LuaIndexExpr,
        mode: LookupMode,
    ) -> Option<LuaType> {
        let prefix = self.type_of_expr(index.get_prefix_expr()?.get_syntax_id());
        let key = match index.get_index_key()? {
            LuaIndexKey::Name(n) => {
                LuaType::StringConst(smol_str::SmolStr::new(n.get_name_text()).into())
            }
            LuaIndexKey::String(s) => {
                LuaType::StringConst(smol_str::SmolStr::new(s.get_value()).into())
            }
            LuaIndexKey::Integer(n) => match n.get_number_value() {
                emmylua_parser::NumberResult::Int(n) => LuaType::IntegerConst(n),
                _ => return None,
            },
            LuaIndexKey::Expr(expr) => {
                self.type_of_expr_at(expr.get_syntax_id(), expr.get_position())
            }
            _ => return None,
        };
        lookup_inner(self, &prefix, &key, mode, &mut Vec::new())
    }
}

impl SemanticModel<'_> {
    pub(crate) fn dictionary_field_type(&self, owner: &LuaType, key: &LuaType) -> Option<LuaType> {
        lookup(self, owner, key, false)
    }

    pub(crate) fn table_assignment_context(
        &self,
        table: &emmylua_parser::LuaTableExpr,
    ) -> Option<LuaType> {
        // Every recursion moves to a containing table, so there is no cyclic lookup.
        if let Some(decl) = self.file_facts()?.decl_by_value_range(table.get_range()) {
            return self.type_of_decl(&decl.id);
        }
        if let Some(assign) = table.get_parent::<emmylua_parser::LuaAssignStat>() {
            let (vars, values) = assign.get_var_and_expr_list();
            let position = values
                .iter()
                .position(|value| value.get_range() == table.get_range())?;
            return Some(self.assignment_target_type(vars.get(position)?.to_expr()));
        }
        let field = table.get_parent::<emmylua_parser::LuaTableField>()?;
        let outer = field.get_parent::<emmylua_parser::LuaTableExpr>()?;
        let typ = self.table_assignment_context(&outer)?;
        let key = match field.get_field_key()? {
            LuaIndexKey::Name(n) => LuaMemberKey::Name(n.get_name_text().into()),
            LuaIndexKey::String(s) => LuaMemberKey::Name(s.get_value().into()),
            _ => return None,
        };
        if let Some((_, write)) = self.directional_member(&typ, &key) {
            return Some(write.typ);
        }
        lookup(self, &typ, &key_type(&key)?, false).or_else(|| self.member_type(&typ, &key))
    }
}
