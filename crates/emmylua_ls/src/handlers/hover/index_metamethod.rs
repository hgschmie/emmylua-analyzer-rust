use emmylua_code_analysis::{LuaMemberKey, SemanticDatabase, SemanticId, SemanticModel};
use emmylua_parser::{LuaAstNode, LuaClosureExpr, LuaComment, LuaIndexExpr, LuaSyntaxToken};
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};
use rowan::TextRange;

use super::{
    desc::{self, HoverDescription},
    render::humanize,
};
use crate::handlers::common::{
    find_index_metamethod_origin, resolve_alias_origin, semantic_id_file,
};

pub(super) fn build_index_metamethod_hover(
    model: &SemanticModel,
    db: &SemanticDatabase,
    token: &LuaSyntaxToken,
    range: TextRange,
) -> Option<Hover> {
    // Concrete declarations always keep their ordinary hover.
    if model.find_decl(token.clone().into()).is_some() {
        return None;
    }
    let origin = find_index_metamethod_origin(model, token)?;
    let index = token.parent_ancestors().find_map(LuaIndexExpr::cast)?;
    let typ = model.type_of_expr(index.get_syntax_id());
    let mut wrapper_docs = None;
    if let Some(wrapper) = model.get_index_metamethod_wrapper(index) {
        let tree = model.syntax_tree_of(wrapper.file_id)?;
        let closure = LuaClosureExpr::cast(wrapper.value.to_node_from_root(&tree.get_red_root())?)?;
        let comments: Vec<_> = tree
            .get_red_root()
            .descendants()
            .filter_map(LuaComment::cast)
            .filter(|comment| {
                comment
                    .get_owner()
                    .is_some_and(|owner| owner.get_range() == closure.get_range())
            })
            .collect();
        let mut description = desc::comments_description(&comments);
        description
            .tags
            .extend(desc::signature_tags_from_comments(&comments));
        if has_docs(&description) {
            wrapper_docs = Some(description);
        }
    }
    let resolved = resolve_alias_origin(model, &origin).unwrap_or_else(|| origin.clone());
    let mut owner = origin.clone();
    let mut description = description(model, &origin)?;
    if !has_docs(&description) {
        owner = resolved;
        description = self::description(model, &owner)?;
    }
    let borrowed = wrapper_docs.is_none();
    if let Some(wrapper) = wrapper_docs {
        description = wrapper;
    }
    if !has_docs(&description) {
        return None;
    }
    let mut value = format!("```lua\n{}\n```", humanize(model, &typ));
    if borrowed && let Some(name) = source_name(model, &owner) {
        value.push_str(&format!("\n\nDocumentation from `{name}`:"));
    }
    if let Some(text) = description.text {
        value.push_str("\n\n");
        value.push_str(&text);
    }
    for tag in description.tags {
        value.push_str("\n\n");
        value.push_str(&tag);
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: db.document(model.file_id())?.to_lsp_range(range),
    })
}

fn has_docs(desc: &HoverDescription) -> bool {
    desc.text.is_some() || !desc.tags.is_empty()
}
fn description(model: &SemanticModel, id: &SemanticId) -> Option<HoverDescription> {
    let foreign = model.model_for(semantic_id_file(id)?);
    let (mut desc, tags) = match id {
        SemanticId::Member(_) => (
            desc::member_description(&foreign, id),
            desc::member_signature_tags(&foreign, id),
        ),
        SemanticId::Decl(_) => (
            desc::decl_description(&foreign, id),
            desc::decl_signature_tags(&foreign, id),
        ),
        _ => return None,
    };
    desc.tags.extend(tags);
    Some(desc)
}
fn source_name(model: &SemanticModel, id: &SemanticId) -> Option<String> {
    match id {
        SemanticId::Member(key) => {
            let tree = model.syntax_tree_of(key.file_id)?;
            let root = tree.get_red_root();
            let token = root.token_at_offset(key.key_range.start()).right_biased()?;
            if let Some(index) = token.parent_ancestors().find_map(LuaIndexExpr::cast) {
                return Some(index.syntax().text().to_string());
            }
            match &model.file_facts_of(key.file_id)?.member_by_id(id)?.key {
                LuaMemberKey::Name(name) => Some(name.to_string()),
                LuaMemberKey::Integer(n) => Some(format!("[{n}]")),
                _ => None,
            }
        }
        SemanticId::Decl(key) => Some(
            model
                .file_facts_of(key.file_id)?
                .decl_by_id(id)?
                .name
                .to_string(),
        ),
        _ => None,
    }
}
