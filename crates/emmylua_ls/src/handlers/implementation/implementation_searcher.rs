use std::sync::Mutex;

use emmylua_code_analysis::{
    DeclKind, LuaType, OwnerId, SemanticDatabase, SemanticId, SemanticModel,
};
use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaDocTagField, LuaExpr, LuaFuncStat, LuaIndexExpr, LuaStat,
    LuaSyntaxToken, LuaTableField,
};
use lsp_types::Location;

use crate::handlers::common::{decl_reference_ranges, member_reference_ranges};

pub fn search_implementations(
    model: &SemanticModel<'_>,
    db: &SemanticDatabase,
    token: LuaSyntaxToken,
) -> Option<Vec<Location>> {
    let mut result = Vec::new();
    let semantic_decl = model.find_decl(token.into())?;
    match &semantic_decl {
        SemanticId::TypeDef(key) => {
            for def in model.type_defs_in_scope(key.scope, &key.full_name).iter() {
                push_location(db, def.file_id, def.name_range, &mut result);
            }
        }
        SemanticId::Member(_) => {
            search_member_implementations(db, &semantic_decl, &mut result);
        }
        SemanticId::Decl(_) => {
            search_decl_implementations(db, &semantic_decl, &mut result);
        }
        _ => {}
    }
    Some(result)
}

fn search_member_implementations(
    db: &SemanticDatabase,
    member: &SemanticId,
    result: &mut Vec<Location>,
) -> Option<()> {
    let SemanticId::Member(key) = member else {
        return None;
    };
    let member_model = SemanticModel::new(db, key.file_id);
    let mut ranges = member_reference_ranges(db, member, true);
    // Include other definitions only when both the key and receiver identity match.
    if let Some(target) = member_model.file_facts()?.member_by_id(member) {
        // Initializer fields also have a synthetic table owner; retain the
        // declaration owner of that same source field and its annotated type.
        let owners: Vec<_> = member_model
            .members()?
            .iter()
            .filter(|candidate| candidate.id == *member)
            .flat_map(|candidate| implementation_owner_ids(&member_model, &candidate.owner))
            .collect();
        let discovered = Mutex::new(Vec::new());
        db.parallel_for_each_file(|file_id, model| {
            let Some(members) = model.members() else {
                return;
            };
            let mut discovered = discovered.lock().expect("implementation locations lock");
            for m in members.iter() {
                if m.key == target.key
                    && implementation_owner_ids(model, &m.owner)
                        .iter()
                        .any(|owner| owners.contains(owner))
                    && let Some(key_range) = m.id.member_key_range()
                    && !discovered.contains(&(file_id, key_range))
                {
                    discovered.push((file_id, key_range));
                }
            }
        });
        let mut discovered = discovered
            .into_inner()
            .expect("implementation locations lock");
        discovered.sort_by_key(|(file_id, _)| file_id.id);
        ranges.extend(discovered);
    }
    let mut signatures = Vec::new();
    let mut others = Vec::new();
    for (file_id, range) in ranges {
        let model = SemanticModel::new(db, file_id);
        if !is_implementation_position(&model, range) {
            continue;
        }
        // Signature implementations (method definitions) come first.
        let is_signature = is_signature_position(&model, range);
        let target = if is_signature {
            &mut signatures
        } else {
            &mut others
        };
        push_location(db, file_id, range, target);
    }
    signatures.append(&mut others);
    result.append(&mut signatures);
    Some(())
}

/// Whether a position is an implementation position: an index key that is a `function T:m()` method name or a `T.x = v` lvalue.
fn is_implementation_position(model: &SemanticModel<'_>, range: rowan::TextRange) -> bool {
    let Some(chunk) = model.chunk() else {
        return false;
    };
    let Some(token) = chunk.syntax().token_at_offset(range.start()).right_biased() else {
        return false;
    };
    let Some(parent) = token.parent() else {
        return false;
    };
    if let Some(index_expr) = LuaIndexExpr::cast(parent.clone()) {
        let Some(stat) = index_expr.ancestors::<LuaStat>().next() else {
            return false;
        };
        if matches!(stat, LuaStat::FuncStat(_)) {
            return true;
        }
        if let LuaStat::AssignStat(assign_stat) = &stat {
            let (vars, _) = assign_stat.get_var_and_expr_list();
            return vars.iter().any(|var| {
                var.syntax()
                    .text_range()
                    .contains(index_expr.syntax().text_range().start())
            });
        }
        return false;
    }
    // `@field x` / table field definition positions.
    if LuaDocTagField::can_cast(parent.kind().into()) {
        return true;
    }
    if let Some(table_field) = LuaTableField::cast(parent.clone()) {
        return table_field.is_assign_field();
    }
    false
}

/// Whether this is a method signature definition (a function member definition position).
fn is_signature_position(model: &SemanticModel<'_>, range: rowan::TextRange) -> bool {
    let Some(chunk) = model.chunk() else {
        return false;
    };
    let Some(token) = chunk.syntax().token_at_offset(range.start()).right_biased() else {
        return false;
    };
    let Some(parent) = token.parent() else {
        return false;
    };
    if let Some(index_expr) = LuaIndexExpr::cast(parent.clone()) {
        return index_expr.ancestors::<LuaFuncStat>().next().is_some();
    }
    false
}

fn search_decl_implementations(
    db: &SemanticDatabase,
    decl: &SemanticId,
    result: &mut Vec<Location>,
) -> Option<()> {
    // Implementation positions are declaration names plus assignment lvalues; plain reads are not implementations.
    let ranges = decl_reference_ranges(db, decl, true);
    for (file_id, range) in ranges {
        let model = SemanticModel::new(db, file_id);
        if is_decl_implementation_position(&model, decl, range) {
            push_location(db, file_id, range, result);
        }
    }
    // Same-name globals: partial classes / global variables may each use `x = {}` in multiple files,
    // and these assignments are implementation positions for the same global name.
    if let SemanticId::Decl(decl_key) = decl {
        let model = SemanticModel::new(db, decl_key.file_id);
        let decl_info = model.file_facts()?.decl_by_id(decl)?;
        if matches!(decl_info.kind, DeclKind::Global) {
            let name = decl_info.name.clone();
            let matched = Mutex::new(Vec::new());
            db.parallel_for_each_file(|file_id, file_model| {
                let Some(facts) = file_model.file_facts() else {
                    return;
                };
                let mut matched = matched.lock().expect("implementation locations lock");
                for d in &facts.decls {
                    if d.name == name && matches!(d.kind, DeclKind::Global) {
                        matched.push((file_id, d.name_range));
                    }
                }
            });
            let mut matched = matched.into_inner().expect("implementation locations lock");
            matched.sort_by_key(|(file_id, _)| file_id.id);
            for (file_id, range) in matched {
                if let Some(location) = make_location(db, file_id, range)
                    && !result.contains(&location)
                {
                    result.push(location);
                }
            }
        }
    }
    Some(())
}

/// Whether a decl reference position is an "assignment implementation position" (the target of `x = v`, or the declaration name itself).
fn is_decl_implementation_position(
    model: &SemanticModel<'_>,
    decl: &SemanticId,
    range: rowan::TextRange,
) -> bool {
    if let SemanticId::Decl(key) = decl
        && range == key.name_range
    {
        return true;
    }
    let Some(chunk) = model.chunk() else {
        return false;
    };
    let Some(token) = chunk.syntax().token_at_offset(range.start()).right_biased() else {
        return false;
    };
    let Some(parent) = token.parent() else {
        return false;
    };
    let Some(LuaExpr::NameExpr(name_expr)) = LuaExpr::cast(parent) else {
        return false;
    };
    let Some(assign) = name_expr.syntax().parent().and_then(LuaAssignStat::cast) else {
        return false;
    };
    let (vars, _) = assign.get_var_and_expr_list();
    vars.iter()
        .any(|var| var.to_expr().get_syntax_id() == name_expr.get_syntax_id())
}

/// Receiver identities, including a local table's explicitly declared class.
fn implementation_owner_ids(model: &SemanticModel<'_>, owner: &SemanticId) -> Vec<OwnerId> {
    let mut owners = model.resolve_owner_ids(owner);
    if matches!(owner, SemanticId::Decl(_)) {
        let ty = model.type_of_decl(owner);
        let type_id = match &ty {
            Some(LuaType::Ref(id) | LuaType::Def(id)) => Some(id),
            Some(LuaType::Generic(generic)) => Some(generic.get_base_type_id_ref()),
            _ => None,
        };
        if let Some(def) = type_id.and_then(|id| model.type_def_of(id)) {
            owners.extend(model.resolve_owner_ids(&def.id));
        }
    }
    owners
}

fn make_location(
    db: &SemanticDatabase,
    file_id: emmylua_code_analysis::FileId,
    range: rowan::TextRange,
) -> Option<Location> {
    let document = db.document(file_id)?;
    let uri = document.get_uri()?;
    let lsp_range = document.to_lsp_range(range)?;
    Some(Location {
        uri,
        range: lsp_range,
    })
}

fn push_location(
    db: &SemanticDatabase,
    file_id: emmylua_code_analysis::FileId,
    range: rowan::TextRange,
    result: &mut Vec<Location>,
) {
    let Some(location) = make_location(db, file_id, range) else {
        return;
    };
    if !result.contains(&location) {
        result.push(location);
    }
}
