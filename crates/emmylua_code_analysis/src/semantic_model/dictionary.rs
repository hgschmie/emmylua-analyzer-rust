//! Dictionary lookups keep declared values intact; only a read can miss a key.
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
    lookup_inner(model, owner, key, nullable, &mut Vec::new())
}
fn lookup_inner(
    model: &SemanticModel,
    owner: &LuaType,
    key: &LuaType,
    nullable: bool,
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
        let normalized = normalize_key(model, key, &mut Vec::new());
        if let LuaType::Union(union) = normalized {
            let values = union
                .into_vec()
                .iter()
                .map(|key| lookup_atomic(model, owner, key, nullable, seen).unwrap_or(LuaType::Nil))
                .collect();
            seen.pop();
            return Some(LuaType::from_vec(values));
        }
        if normalized == LuaType::Nil {
            seen.pop();
            return Some(LuaType::Nil);
        }
        let value = lookup_atomic(model, owner, &normalized, nullable, seen);
        seen.pop();
        return value;
    }
    let value = lookup_atomic(model, owner, key, nullable, seen);
    seen.pop();
    value
}

fn normalize_key(model: &SemanticModel, key: &LuaType, seen: &mut Vec<LuaType>) -> LuaType {
    if seen.len() >= 32 || seen.contains(key) {
        return key.clone();
    }
    seen.push(key.clone());
    let result = match key {
        LuaType::Ref(id) | LuaType::Def(id) => model
            .type_def_of(id)
            .filter(|def| def.kind == TypeDefKind::Alias)
            .and_then(|def| model.alias_target(&def))
            .map(|target| normalize_key(model, &target, seen))
            .unwrap_or_else(|| key.clone()),
        LuaType::Union(union) => LuaType::from_vec(
            union
                .into_vec()
                .iter()
                .map(|key| normalize_key(model, key, seen))
                .collect(),
        ),
        LuaType::MultiLineUnion(union) => normalize_key(model, &union.to_union(), seen),
        _ => key.clone(),
    };
    seen.pop();
    result
}

fn lookup_atomic(
    model: &SemanticModel,
    owner: &LuaType,
    key: &LuaType,
    nullable: bool,
    seen: &mut Vec<LuaType>,
) -> Option<LuaType> {
    match owner {
        LuaType::TableGeneric(params) if params.len() == 2 => {
            Some(signature(model, &params[0], &params[1], key, nullable))
        }
        LuaType::Generic(generic)
            if generic.get_base_type_id().get_name() == "table"
                && generic.get_params().len() == 2 =>
        {
            let params = generic.get_params();
            Some(signature(model, &params[0], &params[1], key, nullable))
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
                    .filter(|(declared, _)| super::type_check::is_compatible(model, declared, key))
                    .map(|(_, value)| result(model, value, nullable))
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
            .and_then(|target| lookup_inner(model, &target, key, nullable, seen)),
        LuaType::Generic(_) => {
            let target = type_eval::expand_alias_generic(model, owner);
            if target == *owner {
                None
            } else {
                lookup_inner(model, &target, key, nullable, seen)
            }
        }
        LuaType::Union(union) => {
            let values: Vec<_> = union
                .into_vec()
                .iter()
                .filter_map(|owner| lookup_inner(model, owner, key, nullable, seen))
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
    nullable: bool,
) -> LuaType {
    if super::type_check::is_compatible(model, declared, key) {
        result(model, value, nullable)
    } else {
        LuaType::Nil
    }
}

impl SemanticModel<'_> {
    pub fn dictionary_index_type(&self, index: &LuaIndexExpr, nullable: bool) -> Option<LuaType> {
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
        lookup(self, &prefix, &key, nullable)
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
