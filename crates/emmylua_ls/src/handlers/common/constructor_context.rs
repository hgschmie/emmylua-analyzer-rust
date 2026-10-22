use crate::handlers::completion::providers::callable_candidates;
use emmylua_code_analysis::{LuaMemberKey, LuaType, MemberInfo, SemanticModel, TypeDefKind};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaCallExpr, LuaIndexKey, LuaLocalStat, LuaSyntaxToken,
    LuaTableExpr, LuaTableField, NumberResult,
};

/// Project the contract of a constructor entry without adding lookup nullability.
pub(crate) fn constructor_field_infos(
    model: &SemanticModel<'_>,
    owner: &LuaType,
    field: &LuaTableField,
) -> Vec<MemberInfo> {
    let Some(key) = field.get_field_key() else {
        return Vec::new();
    };
    let key_type = match &key {
        LuaIndexKey::Name(name) => LuaMemberKey::Name(name.get_name_text().into())
            .to_index_type()
            .unwrap(),
        LuaIndexKey::String(value) => LuaMemberKey::Name(value.get_value().into())
            .to_index_type()
            .unwrap(),
        LuaIndexKey::Integer(value) => match value.get_number_value() {
            NumberResult::Int(value) => LuaType::IntegerConst(value),
            NumberResult::Uint(value) => LuaType::IntegerConst(value as i64),
            NumberResult::Float(value) => LuaType::FloatConst(value),
            _ => return Vec::new(),
        },
        LuaIndexKey::Idx(value) => LuaType::IntegerConst(*value as i64),
        LuaIndexKey::Expr(expr) => model.type_of_expr(expr.get_syntax_id()),
    };
    let member_key = match &key_type {
        LuaType::StringConst(value) | LuaType::DocStringConst(value) => {
            Some(LuaMemberKey::Name(value.as_str().into()))
        }
        LuaType::IntegerConst(value) | LuaType::DocIntegerConst(value) => {
            Some(LuaMemberKey::Integer(*value))
        }
        _ => None,
    };
    constructor_context_infos(model, owner, &key_type, &member_key, &mut Vec::new())
}

fn constructor_context_infos(
    model: &SemanticModel<'_>,
    owner: &LuaType,
    key: &LuaType,
    member_key: &Option<LuaMemberKey>,
    path: &mut Vec<LuaType>,
) -> Vec<MemberInfo> {
    if path.len() >= 64 || path.contains(owner) {
        return Vec::new();
    }
    path.push(owner.clone());
    let value_info = |expected_key: &LuaType, value: &LuaType| {
        if key.is_nil() || !model.type_check(&strip_nullable(key), expected_key) {
            return Vec::new();
        }
        vec![MemberInfo {
            key: member_key
                .clone()
                .unwrap_or_else(|| LuaMemberKey::Name("[index]".into())),
            typ: value.clone(),
            index_key_type: Some(key.clone()),
            id: None,
            file_id: None,
            is_method: false,
        }]
    };
    let named_infos = || {
        member_key
            .as_ref()
            .map(|key| model.member_infos_with_key_all(owner, key))
            .unwrap_or_default()
    };
    let expanded = model.expand_alias_for_hover(owner);
    let result = if expanded != *owner {
        constructor_context_infos(model, &expanded, key, member_key, path)
    } else {
        match owner {
            LuaType::Ref(id) | LuaType::Def(id) => {
                if let Some(def) = model
                    .type_def_of(id)
                    .filter(|def| def.kind == TypeDefKind::Alias)
                    && let Some(ty) = model.alias_target(&def)
                {
                    constructor_context_infos(model, &ty, key, member_key, path)
                } else {
                    named_infos()
                }
            }
            LuaType::Generic(generic)
                if generic.get_base_type_id().get_name() == "table"
                    && generic.get_params().len() == 2 =>
            {
                let params = generic.get_params();
                value_info(&params[0], &params[1])
            }
            LuaType::TableGeneric(params) if params.len() == 2 => {
                value_info(&params[0], &params[1])
            }
            LuaType::Array(array) => value_info(&LuaType::Integer, array.get_base()),
            LuaType::Object(object) => {
                let named = named_infos();
                if !named.is_empty() {
                    named
                } else {
                    object
                        .get_index_access()
                        .iter()
                        .flat_map(|(index, ty)| value_info(index, ty))
                        .collect()
                }
            }
            LuaType::Union(union) => union
                .iter()
                .flat_map(|ty| constructor_context_infos(model, ty, key, member_key, path))
                .collect(),
            _ => named_infos(),
        }
    };
    path.pop();
    result
}

pub(crate) fn table_context_type_for_token(
    model: &SemanticModel<'_>,
    token: &LuaSyntaxToken,
) -> Option<LuaType> {
    let field = token.parent_ancestors().find_map(LuaTableField::cast)?;
    // Context belongs to the constructor key, not arbitrary members inside
    // its value expression or callback body.
    let key = field.get_field_key()?;
    if matches!(key, LuaIndexKey::Expr(_) | LuaIndexKey::Idx(_)) {
        return None;
    }
    let key_range = key.get_range()?;
    if !key_range.contains_range(token.text_range()) {
        return None;
    }
    let table_expr = field.ancestors::<LuaTableExpr>().next()?;
    table_context_type_for_expr(model, &table_expr)
}

pub(crate) fn table_context_type_for_expr(
    model: &SemanticModel<'_>,
    table_expr: &LuaTableExpr,
) -> Option<LuaType> {
    let syntax_id = table_expr.get_syntax_id();

    // A nested constructor inherits the selected value contract of its parent.
    // Walk only direct fields: a callback body must not inherit its table's type.
    if let Some(field) = table_expr.get_parent::<LuaTableField>() {
        if let Some(facts) = model.file_facts()
            && let Some(member) = facts.member_by_value_syntax(syntax_id)
            && let Some(syntax) = member.doc_type_syntax
        {
            return Some(model.doc_type_lua_rich_in(model.file_id(), syntax));
        }
        if let Some(parent) = field.get_parent::<LuaTableExpr>()
            && let Some(owner) = table_context_type_for_expr(model, &parent)
        {
            let types = constructor_field_infos(model, &owner, &field)
                .into_iter()
                .map(|info| info.typ)
                .collect::<Vec<_>>();
            if !types.is_empty() {
                return Some(LuaType::from_vec(types));
            }
        }
    }

    // `---@type T local t = { ... }`: type from the local declaration.
    if let Some(local) = table_expr.ancestors::<LuaLocalStat>().next() {
        if let Some(name) = local
            .get_value_exprs()
            .position(|expr| expr.get_syntax_id() == syntax_id)
            .and_then(|index| local.get_local_name_list().nth(index))
            && let Some(decl) = model.decl_by_offset(name.get_position())
        {
            if let Some(ty) = model.type_of_decl(&decl)
                && !matches!(ty, LuaType::Unknown)
            {
                return Some(ty);
            }
        }
    }

    // `x = { ... }`: type of the assignment target.
    if let Some(assign) = table_expr.ancestors::<LuaAssignStat>().next() {
        let (vars, values) = assign.get_var_and_expr_list();
        if let Some(index) = values
            .iter()
            .position(|expr| expr.get_syntax_id() == syntax_id)
            && let Some(var) = vars.get(index)
        {
            let ty = model.type_of_expr(var.to_expr().get_syntax_id());
            if !matches!(ty, LuaType::Unknown) {
                return Some(ty);
            }
        }
    }

    // Call argument: `observe({ ... })` -> parameter type.
    if let Some(call) = table_expr.ancestors::<LuaCallExpr>().next() {
        let arg_idx = call
            .get_args_list()?
            .get_args()
            .enumerate()
            .find(|(_, arg)| arg.get_syntax_id() == syntax_id)
            .map(|(idx, _)| idx)?;
        // Prefer call-site generic projection when available (`observe("x", { ... })` with `T -> string`).
        if let Some(fun) = model.inferred_call_doc_function(call.get_syntax_id()) {
            let mut param_idx = arg_idx;
            if call.is_colon_call() && !fun.is_colon_define() {
                param_idx += 1;
            }
            if let Some(ty) = fun
                .get_params()
                .get(param_idx)
                .and_then(|(_, ty)| ty.clone())
                && !matches!(ty, LuaType::Unknown)
            {
                return Some(ty);
            }
        }
        let prefix = call.get_prefix_expr()?;
        let mut types = Vec::new();
        for func in callable_candidates(model, &prefix) {
            let mut param_idx = arg_idx;
            if call.is_colon_call() && !func.is_colon_define() {
                param_idx += 1;
            }
            if let Some(ty) = func
                .get_params()
                .get(param_idx)
                .and_then(|(_, ty)| ty.clone())
            {
                if !types.contains(&ty) {
                    types.push(ty);
                }
            }
        }
        if let Some(ty) = types.into_iter().next() {
            return Some(ty);
        }
    }

    let ty = model.type_of_expr(syntax_id);
    (!matches!(ty, LuaType::Unknown)).then_some(ty)
}

/// Remove `nil` from nullable types (`T | nil` / `T?`) for display when the current assignment is non-nil.
pub(crate) fn strip_nullable(ty: &LuaType) -> LuaType {
    match ty {
        LuaType::Union(union) => {
            let mut types = union.into_vec();
            types.retain(|t| !matches!(t, LuaType::Nil));
            LuaType::from_vec(types)
        }
        LuaType::MultiLineUnion(union) => strip_nullable(&union.to_union()),
        _ => ty.clone(),
    }
}
