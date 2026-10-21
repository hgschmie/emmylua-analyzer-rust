//! Index contracts inherited from dictionary supertypes, with generic arguments retained.
use emmylua_parser::{LuaAstNode, LuaDocTagClass};

use super::{SemanticModel, infer::unify, type_check, type_eval};
use crate::{GenericTplId, LuaType, SemanticId, TypeDefKind};

pub(crate) fn dictionaries(model: &SemanticModel<'_>, owner: &LuaType) -> Vec<LuaType> {
    if let Some(types) = model.cache.borrow().inherited_dictionaries.get(owner) {
        return types.clone();
    }
    let mut types = Vec::new();
    collect(model, owner, &mut Vec::new(), &mut types);
    model
        .cache
        .borrow_mut()
        .inherited_dictionaries
        .insert(owner.clone(), types.clone());
    types
}

fn collect(
    model: &SemanticModel<'_>,
    owner: &LuaType,
    seen: &mut Vec<SemanticId>,
    out: &mut Vec<LuaType>,
) {
    if seen.len() >= 32 {
        return;
    }
    let (def, params) = match owner {
        LuaType::TableGeneric(_) | LuaType::Object(_) => {
            if !out.contains(owner) {
                out.push(owner.clone());
            }
            return;
        }
        LuaType::Generic(generic) => {
            if generic.get_base_type_id().get_name() == "table" {
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
    if seen.contains(&def.id) {
        return;
    }
    seen.push(def.id.clone());
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
    ) || matches!(owner, LuaType::Generic(g) if g.get_base_type_id().get_name() == "table")
    {
        return None;
    }
    let contracts = dictionaries(model, owner);
    if contracts.is_empty() {
        return None;
    }
    lookup_key(model, &contracts, key, &mut Vec::new())
}

fn lookup_key(
    model: &SemanticModel<'_>,
    contracts: &[LuaType],
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
            .map(|key| lookup_key(model, contracts, key, seen))
            .collect::<Option<Vec<_>>>()
            .map(LuaType::from_vec)
    } else if let Some(expanded) = expanded {
        lookup_key(model, contracts, &expanded, seen)
    } else {
        let mut values = Vec::new();
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
