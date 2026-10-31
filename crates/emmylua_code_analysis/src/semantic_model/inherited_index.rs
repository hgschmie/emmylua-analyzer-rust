//! Index contracts inherited from dictionary supertypes, with generic arguments retained.
use emmylua_parser::{LuaAstNode, LuaDocTagClass};

use super::cache::{CacheEntry, QueryCacheScope, TypeEvaluationScope};
use super::{SemanticModel, infer::unify, type_check, type_eval};
use crate::{GenericTplId, LuaType, TypeDefKind};

pub(crate) fn is_standard_table(model: &SemanticModel, id: &crate::LuaTypeDeclId) -> bool {
    *id == crate::LuaTypeDeclId::global("table")
        && model
            .type_def_of(id)
            .is_none_or(|def| model.is_std_file(def.file_id))
}

pub(crate) fn dictionaries(model: &SemanticModel<'_>, owner: &LuaType) -> Vec<LuaType> {
    let key = (model.file_id(), owner.clone());
    {
        let cache = model.cache.borrow();
        if let Some(entry) = cache.inherited_dictionaries.get(&key) {
            return entry.get(&cache).cloned().unwrap_or_default();
        }
    }
    let _query = QueryCacheScope::new(&model.cache);
    let revision = model.cache.borrow().revision();
    model
        .cache
        .borrow_mut()
        .inherited_dictionaries
        .insert(key.clone(), CacheEntry::InProgress);
    let mut types = Vec::new();
    collect(model, owner, &mut Vec::new(), &mut types);
    let mut cache = model.cache.borrow_mut();
    let complete = cache.is_complete(revision);
    cache
        .inherited_dictionaries
        .insert(key, CacheEntry::finish(types.clone(), complete));
    types
}

fn collect(
    model: &SemanticModel<'_>,
    owner: &LuaType,
    seen: &mut Vec<LuaType>,
    out: &mut Vec<LuaType>,
) {
    if seen.contains(owner) {
        return;
    }
    // Share upstream's evaluation budget and incompleteness propagation.
    // Distinct generic instances are distinct contracts, even for one class.
    let Some(evaluation) = TypeEvaluationScope::new(&model.cache) else {
        return;
    };
    if !evaluation.visit(owner) {
        return;
    }
    let (def, params) = match owner {
        LuaType::Object(object) if object.get_index_access().is_empty() => return,
        LuaType::TableGeneric(_) | LuaType::Object(_) => {
            if !out.contains(owner) {
                out.push(owner.clone());
            }
            return;
        }
        LuaType::Generic(generic) => {
            if is_standard_table(model, &generic.get_base_type_id()) {
                if !out.contains(owner) {
                    out.push(owner.clone());
                }
                return;
            }
            let Some(def) = model.type_def_of(&generic.get_base_type_id()) else {
                return;
            };
            (def, generic.get_params().to_vec())
        }
        LuaType::Ref(id) | LuaType::Def(id) => {
            let Some(def) = model.type_def_of(id) else {
                return;
            };
            (def, Vec::new())
        }
        _ => return,
    };
    seen.push(owner.clone());
    if def.kind == TypeDefKind::Alias {
        let target = if matches!(owner, LuaType::Generic(_)) {
            Some(type_eval::expand_alias_generic(model, owner))
        } else {
            model.alias_target(&def)
        };
        if let Some(target) = target {
            collect(model, &target, seen, out);
        }
    } else if def.kind == TypeDefKind::Class {
        let mut bindings = unify::TplBindings::new();
        let mut names = Vec::new();
        for (index, param) in def.generic_params.iter().enumerate() {
            let value = params
                .get(index)
                .cloned()
                .or_else(|| {
                    param.default.or(param.constraint).map(|syntax| {
                        unify::substitute(
                            &model.doc_type_lua_in(def.file_id, syntax, &def.generic_params),
                            &bindings,
                        )
                    })
                })
                .unwrap_or(LuaType::Unknown);
            bindings.insert(GenericTplId::Type(index as u32), value.clone());
            names.push((param.name.to_string(), value));
        }
        let names = names.into_iter().collect();
        if let Some(tree) = model.syntax_tree_of(def.file_id)
            && let Some(token) = tree
                .get_red_root()
                .token_at_offset(def.name_range.start())
                .right_biased()
            && let Some(tag) = token.parent_ancestors().find_map(LuaDocTagClass::cast)
            && let Some(supers) = tag.get_supers()
        {
            for parent in supers.get_types() {
                let typ =
                    model.doc_type_lua_in(def.file_id, parent.get_syntax_id(), &def.generic_params);
                let typ =
                    type_eval::substitute_named_refs(&unify::substitute(&typ, &bindings), &names);
                collect(model, &typ, seen, out);
            }
        }
    }
    seen.pop();
}

pub(crate) fn lookup(model: &SemanticModel<'_>, owner: &LuaType, key: &LuaType) -> Option<LuaType> {
    // This fallback is for named classes. Native tables keep their existing inference path.
    if !matches!(
        owner,
        LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_)
    ) || matches!(owner, LuaType::Generic(g) if is_standard_table(model, &g.get_base_type_id()))
    {
        return None;
    }
    let _query = QueryCacheScope::new(&model.cache);
    let revision = model.cache.borrow().revision();
    let contracts = dictionaries(model, owner);
    if !model.cache.borrow().is_complete(revision) {
        return None;
    }
    if contracts.is_empty() {
        return None;
    }
    let fields = model.member_infos(owner);
    let indexes: Vec<_> = model
        .index_infos(owner)
        .into_iter()
        .filter(|info| info.id.is_some())
        .collect();
    let result = lookup_key(model, &contracts, &fields, &indexes, key, &mut Vec::new());
    model
        .cache
        .borrow()
        .is_complete(revision)
        .then_some(result)
        .flatten()
}

fn lookup_key(
    model: &SemanticModel<'_>,
    contracts: &[LuaType],
    fields: &[super::member::MemberInfo],
    indexes: &[crate::IndexInfo],
    key: &LuaType,
    seen: &mut Vec<LuaType>,
) -> Option<LuaType> {
    if seen.len() >= 32 || seen.contains(key) {
        return None;
    }
    seen.push(key.clone());
    let expanded = match key {
        LuaType::Ref(id) | LuaType::Def(id) => model
            .type_def_of(id)
            .filter(|def| def.kind == TypeDefKind::Alias)
            .and_then(|def| model.alias_target(&def)),
        LuaType::Generic(_) => {
            let target = type_eval::expand_alias_generic(model, key);
            (target != *key).then_some(target)
        }
        LuaType::MultiLineUnion(union) => Some(union.to_union()),
        _ => None,
    };
    let result = if *key == LuaType::Nil {
        Some(LuaType::Nil)
    } else if let LuaType::Union(union) = key {
        union
            .into_vec()
            .iter()
            .map(|key| lookup_key(model, contracts, fields, indexes, key, seen))
            .collect::<Option<Vec<_>>>()
            .map(LuaType::from_vec)
    } else if let Some(expanded) = expanded {
        lookup_key(model, contracts, fields, indexes, &expanded, seen)
    } else {
        // A literal declared field overrides the inherited index signature.
        if let Some(literal) = type_eval::member_key_from_literal(key)
            && let Some(field) = fields.iter().find(|field| field.key == literal)
        {
            seen.pop();
            return Some(field.typ.clone());
        }
        let mut values = Vec::new();
        for field in fields {
            let field_key = match &field.key {
                crate::LuaMemberKey::Name(name) => LuaType::StringConst(name.clone().into()),
                crate::LuaMemberKey::Integer(value) => LuaType::IntegerConst(*value),
                _ => continue,
            };
            // A broad key may select any declared field in its domain as well
            // as an inherited dictionary entry.
            if type_check::is_compatible(model, &field_key, key) && !values.contains(&field.typ) {
                values.push(field.typ.clone());
            }
        }
        let mut declared_covers_key = false;
        for info in indexes {
            let accepts = type_check::is_compatible(model, key, &info.key_type);
            let overlaps = accepts || type_check::is_compatible(model, &info.key_type, key);
            if overlaps && !values.contains(&info.value_type) {
                values.push(info.value_type.clone());
            }
            declared_covers_key |= accepts && !matches!(key, LuaType::Any | LuaType::Unknown);
        }
        // Explicit index declarations take precedence over inherited native
        // contracts for a fully covered key domain. A narrower declaration is
        // still one possible value for a broad key, alongside inherited entries.
        if declared_covers_key {
            seen.pop();
            return (!values.is_empty()).then(|| LuaType::from_vec(values));
        }
        for contract in contracts {
            let pairs: Vec<(&LuaType, &LuaType)> = match contract {
                LuaType::TableGeneric(params) if params.len() == 2 => {
                    vec![(&params[0], &params[1])]
                }
                LuaType::Generic(generic) if generic.get_params().len() == 2 => {
                    vec![(&generic.get_params()[0], &generic.get_params()[1])]
                }
                LuaType::Object(object) => object
                    .get_index_access()
                    .iter()
                    .map(|(k, v)| (k, v))
                    .collect(),
                _ => Vec::new(),
            };
            for (declared, value) in pairs {
                if type_check::is_compatible(model, key, declared) && !values.contains(value) {
                    values.push(value.clone());
                }
            }
        }
        (!values.is_empty()).then(|| LuaType::from_vec(values))
    };
    seen.pop();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VirtualWorkspace;

    #[test]
    fn inherited_dictionary_long_chain_remains_complete() {
        let mut source = String::from("---@class Level0: table<string,integer>\n");
        for n in 1..48 {
            source.push_str(&format!("---@class Level{n}: Level{}\n", n - 1));
        }
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let ty = ws.ty("Level47");
        let model = ws.analysis.semantic_model(file);
        for _ in 0..2 {
            assert_eq!(
                lookup(&model, &ty, &LuaType::String),
                Some(LuaType::Integer)
            );
        }
    }

    #[test]
    fn inherited_dictionary_growing_projection_does_not_cache_partial_values() {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def("---@class Growing<T>: Growing<T[]>, table<string,T>");
        let ty = ws.ty("Growing<integer>");
        let model = ws.analysis.semantic_model(file);
        assert_eq!(lookup(&model, &ty, &LuaType::String), None);
        assert!(model.cache.borrow().inherited_dictionaries.is_empty());
    }
}

#[cfg(test)]
mod field_contract_tests {
    use super::*;
    use crate::VirtualWorkspace;

    #[test]
    fn inherited_dictionary_dynamic_projection_includes_declared_fields() {
        let mut ws = VirtualWorkspace::new();
        let file=ws.def("---@class Person\n---@field name string\n---@class People: table<string,Person>\n---@field count integer");
        let owner = ws.ty("People");
        let person = ws.ty("Person");
        let model = ws.analysis.semantic_model(file);
        for _ in 0..2 {
            assert_eq!(
                lookup(&model, &owner, &LuaType::String),
                Some(LuaType::from_vec(vec![person.clone(), LuaType::Integer]))
            );
            assert_eq!(
                lookup(
                    &model,
                    &owner,
                    &LuaType::StringConst(smol_str::SmolStr::new("count").into())
                ),
                Some(LuaType::Integer)
            );
        }
    }
}

#[cfg(test)]
#[test]
fn inherited_fixed_fields_do_not_create_dictionary_contracts() {
    let mut ws = crate::VirtualWorkspace::new();
    let file =
        ws.def("---@alias Fields {foo:string}\n---@class Fixed: Fields\n---@field bar integer");
    let ty = ws.ty("Fixed");
    let model = ws.analysis.semantic_model(file);
    assert!(dictionaries(&model, &ty).is_empty());
    assert_eq!(lookup(&model, &ty, &LuaType::String), None);
}

#[cfg(test)]
#[test]
fn inherited_dictionary_named_table_requires_standard_identity() {
    let mut ws = crate::VirtualWorkspace::new();
    let file = ws.def("---@class table<K,V>\n---@field fixed string");
    let owner = ws.ty("table<integer,boolean>");
    let model = ws.analysis.semantic_model(file);
    assert!(dictionaries(&model, &owner).is_empty());
    assert_eq!(lookup(&model, &owner, &LuaType::Integer), None);
}

#[cfg(test)]
#[test]
fn inherited_dictionary_keeps_declared_index_signatures() {
    let mut ws = crate::VirtualWorkspace::new();
    let file=ws.def("---@class Child: table<string,integer>\n---@field [integer] boolean\n---@class Override: table<string,integer>\n---@field [string] boolean");
    let child = ws.ty("Child");
    let overridden = ws.ty("Override");
    let model = ws.analysis.semantic_model(file);
    assert_eq!(
        lookup(&model, &child, &LuaType::Integer),
        Some(LuaType::Boolean)
    );
    assert_eq!(
        lookup(&model, &child, &LuaType::String),
        Some(LuaType::Integer)
    );
    assert_eq!(
        lookup(&model, &overridden, &LuaType::String),
        Some(LuaType::Boolean)
    );
}
