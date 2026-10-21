//! Broad index keys project index signatures rather than requiring literal fields.
use super::*;

pub(super) fn dynamic_index_type(
    model: &SemanticModel,
    base: &LuaType,
    key: &LuaType,
) -> Option<LuaType> {
    enum Task {
        Enter(LuaType),
        Leave,
    }
    let mut path = AliasPath::new(model)?;
    let mut pending = vec![Task::Enter(base.clone())];
    let mut types = Vec::new();
    while let Some(task) = pending.pop() {
        let owner = match task {
            Task::Leave => {
                path.leave();
                continue;
            }
            Task::Enter(owner) => owner,
        };
        let expanded = expand_alias_head(model, &owner)?;
        path.enter(&owner)?;
        pending.push(Task::Leave);
        if expanded != owner {
            pending.push(Task::Enter(expanded));
            continue;
        }
        match &owner {
            LuaType::Union(union) => pending.extend(union.iter().cloned().map(Task::Enter)),
            LuaType::TableGeneric(params) => {
                if let [expected, value, ..] = params.as_slice()
                    && model.type_check_subtype(key, expected)
                {
                    types.push(value.clone());
                }
            }
            LuaType::Array(array) if model.type_check_subtype(key, &LuaType::Integer) => {
                types.push(array.get_base().clone());
            }
            LuaType::Object(object) => {
                for (expected, value) in object.get_index_access() {
                    if model.type_check_subtype(key, expected) {
                        types.push(value.clone());
                    }
                }
            }
            LuaType::Ref(id) | LuaType::Def(id) => {
                if let Some(def) = model.type_def_of(id) {
                    for info in model.member_infos(&owner) {
                        if info
                            .index_key_type
                            .as_ref()
                            .is_some_and(|expected| model.type_check_subtype(key, expected))
                        {
                            types.push(info.typ);
                        }
                    }
                    pending.extend(
                        model
                            .base_types(&def.type_decl_id())
                            .into_iter()
                            .map(Task::Enter),
                    );
                }
            }
            LuaType::Generic(generic) => {
                let id = generic.get_base_type_id();
                if id.get_name() == "table" {
                    pending.push(Task::Enter(LuaType::TableGeneric(Rc::new(
                        generic.get_params().to_vec(),
                    ))));
                } else if let Some(def) = model.type_def_of(&id) {
                    for info in model.member_infos(&owner) {
                        if info
                            .index_key_type
                            .as_ref()
                            .is_some_and(|expected| model.type_check_subtype(key, expected))
                        {
                            types.push(info.typ);
                        }
                    }
                    let bindings = generic
                        .get_params()
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| (GenericTplId::Type(i as u32), ty.clone()))
                        .collect();
                    pending.extend(
                        model
                            .base_types(&def.type_decl_id())
                            .into_iter()
                            .map(|base| Task::Enter(unify::substitute(&base, &bindings))),
                    );
                }
            }
            _ => {}
        }
    }
    if types.is_empty() {
        return None;
    }
    // A broad runtime key does not establish that the entry exists.
    types.push(LuaType::Nil);
    Some(LuaType::from_vec(types))
}
