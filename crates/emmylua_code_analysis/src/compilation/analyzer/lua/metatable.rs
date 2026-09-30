use emmylua_parser::{LuaAstNode, LuaCallExpr, LuaExpr, LuaIndexKey, LuaTableField};
use hashbrown::HashSet;

use crate::{
    InFiled, LuaOperator, LuaOperatorMetaMethod, LuaOperatorOwner, LuaSignatureId, OperatorFunction,
};

use super::LuaAnalyzer;

pub fn analyze_setmetatable(analyzer: &mut LuaAnalyzer, call_expr: LuaCallExpr) -> Option<()> {
    let arg_list = call_expr.get_args_list()?;
    let args = arg_list.get_args().collect::<Vec<_>>();

    if args.len() != 2 {
        return Some(());
    }

    let table = args[0].clone();
    let metatable = args[1].clone();
    let LuaExpr::TableExpr(metatable) = metatable else {
        return Some(());
    };

    let file_id = analyzer.file_id;
    analyzer.db.get_metatable_index_mut().add(
        InFiled::new(file_id, table.get_range()),
        InFiled::new(file_id, metatable.get_range()),
    );

    // A named table retains the identity of its constructor. Member lookup uses
    // that identity, not the range of the reference passed to setmetatable.
    if let Some(table_identity) = stable_local_table(analyzer, table.clone()) {
        analyzer.db.get_metatable_index_mut().add(
            InFiled::new(file_id, table_identity.get_range()),
            InFiled::new(file_id, metatable.get_range()),
        );
    }

    let operator_owner = LuaOperatorOwner::Table(InFiled::new(file_id, metatable.get_range()));
    for (field, field_key) in metatable.get_fields_with_keys() {
        analyze_metable_field(analyzer, &field, &field_key, &operator_owner);
    }

    Some(())
}

// Resolve only immutable local bindings and aliases to literal tables. Inferred
// types can retain a declaration's original table after reassignment, so using
// them here would attach the metatable to an unrelated runtime object.
fn stable_local_table(analyzer: &LuaAnalyzer, mut expr: LuaExpr) -> Option<LuaExpr> {
    let references = analyzer
        .db
        .get_reference_index()
        .get_local_reference(&analyzer.file_id)?;
    let tree = analyzer.db.get_vfs().get_syntax_tree(&analyzer.file_id)?;
    let mut visited = HashSet::new();
    loop {
        match expr {
            LuaExpr::TableExpr(_) => return Some(expr),
            LuaExpr::NameExpr(ref name) => {
                let decl_id = references.get_decl_id(&name.get_range())?;
                if !visited.insert(decl_id) || references.get_decl_references(&decl_id)?.mutable {
                    return None;
                }
                let decl = analyzer.db.get_decl_index().get_decl(&decl_id)?;
                if !decl.is_local() || decl.is_param() {
                    return None;
                }
                expr = LuaExpr::cast(decl.get_value_syntax_id()?.to_node(tree)?)?;
            }
            _ => return None,
        }
    }
}

fn analyze_metable_field(
    analyzer: &mut LuaAnalyzer,
    field: &LuaTableField,
    field_key: &LuaIndexKey,
    operator_owner: &LuaOperatorOwner,
) -> Option<()> {
    let field_name = match field_key {
        LuaIndexKey::Name(n) => n.get_name_text().to_string(),
        LuaIndexKey::String(s) => s.get_value(),
        _ => return None,
    };

    let meta_method = LuaOperatorMetaMethod::from_metatable_name(&field_name)?;
    let field_value = field.get_value_expr()?;
    let file_id = analyzer.file_id;

    let signature_id = match field_value {
        LuaExpr::ClosureExpr(closure) => LuaSignatureId::from_closure(file_id, &closure),
        _ => return None,
    };

    let operator = LuaOperator::new(
        operator_owner.clone(),
        meta_method,
        file_id,
        field.get_range(),
        OperatorFunction::Signature(signature_id),
    );
    analyzer.db.get_operator_index_mut().add_operator(operator);

    Some(())
}
