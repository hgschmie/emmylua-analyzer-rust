use super::{desc::member_description, render::humanize};
use emmylua_code_analysis::{SemanticDatabase, SemanticId, SemanticModel};
use emmylua_parser::LuaSyntaxToken;
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};

pub(super) fn hover(
    model: &SemanticModel,
    db: &SemanticDatabase,
    token: &LuaSyntaxToken,
) -> Option<Hover> {
    let pair = super::super::common::directional_index_at(model, token)
        .map(|(read, write, _)| (read, write))
        .or_else(|| model.directional_declaration(&model.find_decl(token.clone().into())?))?;
    let (read, write) = pair;
    let mut value = format!(
        "```lua\nRead: {}\nWrite: {}\n```",
        humanize(model, &read.typ),
        humanize(model, &write.typ)
    );
    if let Some(id) = read.id.as_ref() {
        let desc = member_description(model, id);
        let text = desc.text.or_else(|| {
            let SemanticId::Member(key) = id else {
                return None;
            };
            let tree = model.syntax_tree_of(key.file_id)?;
            let text = tree.get_red_root().text().to_string();
            let before = &text[..u32::from(key.key_range.start()) as usize];
            let mut lines = before
                .lines()
                .rev()
                .skip(1)
                .take_while(|line| line.trim_start().starts_with("--- "))
                .map(|line| line.trim_start().trim_start_matches("--- ").to_owned())
                .collect::<Vec<_>>();
            lines.reverse();
            (!lines.is_empty()).then(|| lines.join("\n"))
        });
        if let Some(text) = text {
            value.push_str("\n\n");
            value.push_str(&text);
        }
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: db
            .document(model.file_id())?
            .to_lsp_range(token.text_range()),
    })
}
