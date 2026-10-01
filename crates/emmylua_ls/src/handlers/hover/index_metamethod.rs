use emmylua_code_analysis::{
    LuaMemberKey, LuaSemanticDeclId, LuaType, SemanticModel, humanize_type,
};
use emmylua_parser::{LuaAstNode, LuaExpr, LuaIndexExpr, LuaSyntaxToken};

use crate::handlers::common::{find_index_metamethod_origin, find_member_origin_owner};

use super::{HoverBuilder, humanize_types::extract_description_from_property_owner};

pub(super) fn build_index_metamethod_hover<'a>(
    model: &'a SemanticModel,
    token: &LuaSyntaxToken,
) -> Option<HoverBuilder<'a>> {
    let origin = find_index_metamethod_origin(model, token)?;
    let index = token.parent_ancestors().find_map(LuaIndexExpr::cast)?;
    // A quoted/integer key's semantic info describes the literal. Hover should
    // describe the indexed value, just as it does for a dotted member name.
    let typ = model.infer_expr(LuaExpr::IndexExpr(index)).ok()?;
    let db = model.get_db();
    let mut builder = HoverBuilder::new(model, Some(token.clone()), false);
    // The __index result may wrap the backing function with different arguments
    // or results. Borrow documentation, never its type or overload signatures.
    builder.set_type_description(humanize_type(db, &typ, builder.detail_render_level));

    let resolved = if let LuaSemanticDeclId::Member(member) = origin {
        find_member_origin_owner(model, member).unwrap_or_else(|| origin.clone())
    } else {
        origin.clone()
    };
    // Documentation on an alias takes precedence over its underlying member.
    let (owner, description) =
        if let Some(description) = extract_description_from_property_owner(model, &origin) {
            (origin, Some(description))
        } else {
            let description = extract_description_from_property_owner(model, &resolved);
            (resolved, description)
        };
    let documented_type = match &owner {
        LuaSemanticDeclId::Member(id) => model.get_type((*id).into()),
        LuaSemanticDeclId::LuaDecl(id) => model.get_type((*id).into()),
        _ => return None,
    };
    // Explicit documentation on the returned wrapper wins. A direct lookup
    // shares its signature with the backing member, so include that member's
    // description as well as its parameter and return documentation.
    if typ != documented_type
        && let LuaType::Signature(signature) = &typ
    {
        let description = extract_description_from_property_owner(
            model,
            &LuaSemanticDeclId::Signature(*signature),
        );
        let has_description = description.is_some();
        builder.add_description_from_info(description);
        builder.add_signature_params_rets_description(typ.clone());
        if has_description || !builder.annotation_description.is_empty() {
            return Some(builder);
        }
    }
    let has_description = description.is_some();
    builder.add_description_from_info(description);
    builder.add_signature_params_rets_description(documented_type);
    if !has_description && builder.annotation_description.is_empty() {
        return None;
    }
    if let Some(name) = documentation_source_name(model, &owner) {
        builder.annotation_description.insert(
            0,
            lsp_types::MarkedString::from_markdown(format!("Documentation from `{name}`:")),
        );
    }
    Some(builder)
}

fn documentation_source_name(model: &SemanticModel, owner: &LuaSemanticDeclId) -> Option<String> {
    let db = model.get_db();
    match owner {
        LuaSemanticDeclId::Member(id) => {
            let member = db.get_member_index().get_member(id)?;
            let tree = db.get_vfs().get_syntax_tree(&id.file_id)?;
            let node = id.get_syntax_id().to_node(tree)?;
            if let Some(index) = LuaIndexExpr::cast(node) {
                return Some(index.syntax().text().to_string());
            }
            match member.get_key() {
                LuaMemberKey::Name(name) => Some(name.to_string()),
                LuaMemberKey::Integer(index) => Some(format!("[{index}]")),
                _ => None,
            }
        }
        LuaSemanticDeclId::LuaDecl(id) => {
            Some(db.get_decl_index().get_decl(id)?.get_name().to_owned())
        }
        _ => None,
    }
}
