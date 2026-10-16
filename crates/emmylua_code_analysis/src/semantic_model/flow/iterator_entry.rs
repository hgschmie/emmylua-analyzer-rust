//! A key yielded by a standard table iterator identifies an existing entry.
use super::{SemanticModel, computed_effects, computed_guard};
use crate::semantic_db::flow::LuaClosureId;
use crate::{LuaMemberKey, LuaType, SemanticId};
use emmylua_parser::{
    LuaAstNode, LuaAstToken, LuaExpr, LuaForRangeStat, LuaIndexExpr, LuaIndexKey,
};
use rowan::TextSize;

pub(super) fn entry_type(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    offset: TextSize,
) -> Option<LuaType> {
    let LuaIndexKey::Expr(LuaExpr::NameExpr(key)) = index.get_index_key()? else {
        return None;
    };
    let key_id = model.resolve_name(key.get_position())?;
    let prefix = index.get_prefix_expr()?;
    let collection = computed_guard::access(model, &prefix)?;
    let expected = computed_guard::access(model, &LuaExpr::IndexExpr(index.clone()))?;
    let closure = LuaClosureId::from_node(index.syntax());
    for loop_stat in index.ancestors::<LuaForRangeStat>() {
        if LuaClosureId::from_node(loop_stat.syntax()) != closure {
            break;
        }
        let first = loop_stat.get_var_name_list().next()?;
        if model.decl_by_offset(first.get_position()).as_ref() != Some(&key_id) {
            continue;
        }
        let body = loop_stat.get_block()?;
        if !body.get_range().contains(offset) {
            continue;
        }
        let mut iterators = loop_stat.get_expr_list();
        let LuaExpr::CallExpr(call) = iterators.next()? else {
            return None;
        };
        if iterators.next().is_some() {
            return None;
        }
        let LuaExpr::NameExpr(callee) = call.get_prefix_expr()? else {
            return None;
        };
        let name = callee.get_name_text()?;
        if !matches!(name.as_str(), "pairs" | "ipairs") {
            return None;
        }
        // A local/custom pairs, or a custom lexical environment, has no such contract.
        if computed_guard::reference(model, &callee) != Some(SemanticId::name(name.clone().into()))
        {
            return None;
        }
        let declarations = model.global_decls_named_primary(name.as_str());
        if declarations.is_empty()
            || declarations.iter().any(|decl| {
                !matches!(decl,
            SemanticId::Decl(id) if model.is_std_file(id.file_id))
            })
        {
            return None;
        }
        let args = call.get_args_list()?;
        let mut args = args.get_args();
        let argument = args.next()?;
        if args.next().is_some()
            || computed_guard::access(model, &argument).as_ref() != Some(&collection)
        {
            return None;
        }
        let owner = model.type_of_expr_at(prefix.get_syntax_id(), offset);
        if !computed_guard::ordinary_collection(model, &owner)
            || model.member_infos(&owner).iter().any(|member| {
                matches!(&member.key,
                LuaMemberKey::Name(name) if matches!(name.as_str(), "__pairs" | "__ipairs" | "__index"))
            })
        {
            return None;
        }
        if computed_effects::iteration_conflicts(model, &expected, body.syntax(), offset) {
            return None;
        }
        let base = match model.type_of_expr(index.get_syntax_id()) {
            LuaType::Unknown => match &owner {
                LuaType::Array(array) => array.get_base().clone(),
                _ => return None,
            },
            ty => ty,
        };
        return Some(super::remove_type(base, &LuaType::Nil));
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::{DiagnosticCode, VirtualWorkspace};

    fn check(body: &str, safe: bool) {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let source = format!(
            r#"
---@class Entry
---@field value integer
---@class State
---@field rows Entry[]
---@param state State
---@param other State
---@param other_key integer
local function test(state, other, other_key)
{body}
end
"#
        );
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &source),
            safe,
            "{body}"
        );
    }

    #[test]
    fn iterator_entry_guarantees_existing_value() {
        for iterator in ["pairs", "ipairs"] {
            for body in [
                "print(state.rows[key].value)",
                "local value = state.rows[key]; print(value.value)",
                "if other_key > 0 then print(state.rows[key].value) end",
                "opaque(); state.rows[key].value = 1; print(state.rows[key].value)",
            ] {
                check(
                    &format!("for key, value in {iterator}(state.rows) do {body} end"),
                    true,
                );
            }
        }
    }

    #[test]
    fn iterator_entry_expires_on_visible_conflicts() {
        for body in [
            "key = other_key; print(state.rows[key].value)",
            "state = other; print(state.rows[key].value)",
            "state.rows = {}; print(state.rows[key].value)",
            "state.rows[key] = nil; print(state.rows[key].value)",
            "local alias = state.rows; alias[key] = nil; print(state.rows[key].value)",
            "rawset(state.rows, key, nil); print(state.rows[key].value)",
            "local function clear() state.rows[key] = nil end; clear(); print(state.rows[key].value)",
            "if other_key > 0 then state.rows[key] = nil end; print(state.rows[key].value)",
            "print(other.rows[key].value)",
            "local key = other_key; print(state.rows[key].value)",
            "local function later() print(state.rows[key].value) end",
            "while other_key > 0 do print(state.rows[key].value); state.rows[key] = nil end",
        ] {
            check(
                &format!("for key, value in pairs(state.rows) do {body} end"),
                false,
            );
        }
    }

    #[test]
    fn iterator_entry_rejects_custom_iterators_and_outside_reads() {
        for source in [
            "print(state.rows[other_key].value)",
            "for key in pairs(state.rows) do end; print(state.rows[other_key].value)",
            "local pairs = function(t) return next, t end; for key in pairs(state.rows) do print(state.rows[key].value) end",
            "for key in custom(state.rows) do print(state.rows[key].value) end",
            "pairs = function(t) return next, t end; for key in pairs(state.rows) do print(state.rows[key].value) end",
            "local _ENV = {}; for key in pairs(state.rows) do print(state.rows[key].value) end",
        ] {
            check(source, false);
        }
    }

    #[test]
    fn custom_pairs_metamethod_does_not_guarantee_entry() {
        use emmylua_parser::{LuaAstNode, LuaIndexExpr};
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file = ws.def(
            r#"
        ---@class Custom: { [integer]: Entry? }
        ---@field __pairs fun(): function, table, nil
        ---@class Entry
        ---@field value integer
        ---@param rows Custom
        local function test(rows)
            for key in pairs(rows) do print(rows[key].value) end
        end
        "#,
        );
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|index| index.syntax().text().to_string() == "rows[key]")
            .unwrap();
        assert!(super::entry_type(&model, &index, index.get_position()).is_none());
    }
}
