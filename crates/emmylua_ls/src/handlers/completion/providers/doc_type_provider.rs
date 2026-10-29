//! Doc type completion: type names in `---@type` / `@param` / `@field` etc.
//!
//! Enumerate visible declarations, including library and metadata types;
//! attribute contexts only suggest classes.

use std::collections::HashSet;

use emmylua_code_analysis::TypeDefKind;
use emmylua_parser::{LuaAstNode, LuaDocAttributeUse, LuaDocNameType, LuaSyntaxKind, LuaTokenKind};
use lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, Range, TextEdit};
use rowan::TextRange;

use crate::handlers::completion::completion_builder::CompletionBuilder;

use super::{CompletionProvider, ProviderDecision};

pub struct DocTypeProvider;

impl CompletionProvider for DocTypeProvider {
    fn name(&self) -> &'static str {
        "doc_type"
    }

    fn supports(&self, builder: &CompletionBuilder) -> bool {
        completion_type_for(builder).is_some()
    }

    fn complete(&self, builder: &mut CompletionBuilder) -> ProviderDecision {
        if complete_provider(builder).is_some() {
            ProviderDecision::Stop
        } else {
            ProviderDecision::NoMatch
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionType {
    Type,
    AttributeUse,
}

fn complete_provider(builder: &mut CompletionBuilder) -> Option<()> {
    if builder.is_cancelled() {
        return None;
    }

    let completion_type = completion_type_for(builder)?;
    // Only text before the cursor filters candidates. Replacing the complete
    // name also removes a stale suffix when completion is invoked mid-token.
    let name_token = if builder.trigger_token.kind() == LuaTokenKind::TkName.into() {
        Some(builder.trigger_token.clone())
    } else {
        // At a token start completion() chooses the token on the left. Replace
        // the adjacent doc name rather than inserting before its stale text.
        builder.trigger_token.next_token().filter(|token| {
            token.kind() == LuaTokenKind::TkName.into()
                && token.text_range().start() == builder.position_offset
                && token
                    .parent()
                    .is_some_and(|parent| LuaDocNameType::cast(parent).is_some())
        })
    };
    let (prefix_content, edit_range) = if let Some(token) = name_token {
        let range = token.text_range();
        // Both the start and end of a name are valid cursor boundaries.
        let length = usize::from(builder.position_offset.checked_sub(range.start())?);
        (token.text().get(..length)?.to_owned(), range)
    } else {
        (String::new(), TextRange::empty(builder.position_offset))
    };
    let edit_range = builder.get_document().to_lsp_range(edit_range)?;
    let prefix = if let Some(last_sep) = prefix_content.rfind('.') {
        let (path, _) = prefix_content.split_at(last_sep + 1);
        path
    } else {
        ""
    };
    let partial = &prefix_content[prefix.len()..];
    complete_types_by_prefix(builder, prefix, partial, edit_range, completion_type);
    Some(())
}

fn complete_types_by_prefix(
    builder: &mut CompletionBuilder,
    prefix: &str,
    partial: &str,
    edit_range: Range,
    completion_type: CompletionType,
) {
    let mut seen = HashSet::new();
    let mut file_ids = builder.semantic_model.file_ids();
    file_ids.sort();
    for file_id in file_ids {
        if builder.is_cancelled() {
            return;
        }
        let model = builder.semantic_model.model_for(file_id);
        let Some(exports) = model.file_exports_current() else {
            continue;
        };
        for def in exports.types.iter() {
            let full_name = def.full_name.as_str();
            // Prefix path matching: `ns.` only accepts types under `ns`.
            if !full_name.starts_with(prefix) {
                continue;
            }
            let relative = &full_name[prefix.len()..];
            if !relative.starts_with(partial) {
                continue;
            }
            if completion_type == CompletionType::AttributeUse && def.kind != TypeDefKind::Class {
                continue;
            }
            // Use the same scope/shadowing rules as a reference to the name.
            // Metadata denotes declarations, not hidden completion candidates.
            if seen.contains(full_name)
                || !builder
                    .semantic_model
                    .resolve_type_def(full_name)
                    .is_some_and(|visible| visible.id == def.id)
            {
                continue;
            }
            seen.insert(full_name.to_string());
            let kind = match completion_type {
                CompletionType::AttributeUse => CompletionItemKind::CLASS,
                CompletionType::Type => match def.kind {
                    TypeDefKind::Enum => CompletionItemKind::ENUM,
                    TypeDefKind::Class => CompletionItemKind::CLASS,
                    TypeDefKind::Alias => CompletionItemKind::STRUCT,
                },
            };
            builder.add_completion_item(CompletionItem {
                label: relative.to_string(),
                kind: Some(kind),
                filter_text: Some(full_name.to_string()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: edit_range,
                    new_text: full_name.to_string(),
                })),
                ..Default::default()
            });
        }
    }
}

fn completion_type_for(builder: &CompletionBuilder) -> Option<CompletionType> {
    match builder.trigger_token.kind().into() {
        LuaTokenKind::TkName => {
            let parent = builder.trigger_token.parent()?;
            let doc_name = LuaDocNameType::cast(parent)?;
            if doc_name.get_parent::<LuaDocAttributeUse>().is_some() {
                return Some(CompletionType::AttributeUse);
            }
            Some(CompletionType::Type)
        }
        LuaTokenKind::TkWhitespace => {
            let left_token = builder.trigger_token.prev_token()?;
            match left_token.kind().into() {
                LuaTokenKind::TkTagReturn | LuaTokenKind::TkTagType => {
                    return Some(CompletionType::Type);
                }
                LuaTokenKind::TkName => {
                    let parent = left_token.parent()?;
                    match parent.kind().into() {
                        LuaSyntaxKind::DocTagParam
                        | LuaSyntaxKind::DocTagField
                        | LuaSyntaxKind::DocTagAlias
                        | LuaSyntaxKind::DocTagCast => return Some(CompletionType::Type),
                        _ => {}
                    }
                }
                LuaTokenKind::TkComma | LuaTokenKind::TkDocOr => {
                    let parent = left_token.parent()?;
                    if parent.kind() == LuaSyntaxKind::DocTypeList.into() {
                        return Some(CompletionType::Type);
                    }
                }
                LuaTokenKind::TkColon => {
                    let parent = left_token.parent()?;
                    if parent.kind() == LuaSyntaxKind::DocTagClass.into() {
                        return Some(CompletionType::Type);
                    }
                }
                _ => {}
            }
            None
        }
        LuaTokenKind::TkDocAttributeUse => Some(CompletionType::AttributeUse),
        _ => None,
    }
}
