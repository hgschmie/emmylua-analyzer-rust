mod goto_def_definition;
mod goto_label;
mod goto_module_file;
mod goto_string;

use emmylua_code_analysis::{EmmyLuaAnalysis, FileId, SemanticId, SemanticModel};
use emmylua_parser::{LuaAstNode, LuaAstToken, LuaStringToken, LuaSyntaxToken, LuaTokenKind};
use lsp_types::{
    ClientCapabilities, GotoDefinitionParams, GotoDefinitionResponse, OneOf, Position,
    ServerCapabilities,
};
use rowan::TokenAtOffset;
use tokio_util::sync::CancellationToken;

use super::RegisterCapabilities;
use crate::context::{RequestOutcome, ServerContextSnapshot, snapshot_query};

pub async fn on_goto_definition_handler(
    context: ServerContextSnapshot,
    params: GotoDefinitionParams,
    cancel_token: CancellationToken,
) -> RequestOutcome<GotoDefinitionResponse> {
    let uri = params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;
    snapshot_query(context.analysis(), cancel_token, move |analysis| {
        let file_id = analysis.get_file_id(&uri)?;
        definition(analysis, file_id, position)
    })
    .await
}

pub fn definition(
    analysis: &EmmyLuaAnalysis,
    file_id: FileId,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    let model = analysis.semantic_model(file_id);
    let document = analysis.db.document(file_id)?;
    let root = model.chunk()?;
    let position_offset =
        document.get_offset(position.line as usize, position.character as usize)?;

    if position_offset > root.syntax().text_range().end() {
        return None;
    }
    let token = match root.syntax().token_at_offset(position_offset) {
        TokenAtOffset::Single(token) => token,
        TokenAtOffset::Between(left, right) => {
            if left.kind() == LuaTokenKind::TkName.into()
                || (left.kind() == LuaTokenKind::TkLeftBracket.into()
                    && right.kind() == LuaTokenKind::TkInt.into())
            {
                left
            } else {
                right
            }
        }
        TokenAtOffset::None => {
            return None;
        }
    };

    if let Some((read, write, assignment)) = super::common::directional_index_at(&model, &token)
        && let Some(SemanticId::Member(key)) = if assignment { write.id } else { read.id }
    {
        let document = analysis.db.document(key.file_id)?;
        return Some(GotoDefinitionResponse::Scalar(lsp_types::Location::new(
            document.get_uri()?,
            document.to_lsp_range(key.key_range)?,
        )));
    }

    // 1. Label definition (goto / label).
    if let Some(response) = goto_label::goto_label_definition(&model, &analysis.db, &token) {
        return Some(response);
    }

    if model.find_decl(token.clone().into()).is_none()
        && let Some(origin) = find_index_metamethod_definition(&model, &token)
    {
        return goto_def_definition::goto_def_definition(&model, &analysis.db, &origin, &token);
    }

    // 2. String token: require module file / string template reference.
    if let Some(string_token) = LuaStringToken::cast(token.clone()) {
        if let Some(response) =
            goto_module_file::goto_module_file(&analysis.db, file_id, string_token.clone())
        {
            return Some(response);
        }
        if let Some(response) =
            goto_string::goto_str_tpl_ref_definition(&model, &analysis.db, string_token)
        {
            return Some(response);
        }
        return None;
    }

    // 2.5 Doc description reference / `@see`.
    if let Some(response) = goto_def_definition::goto_doc_definition(
        &model,
        &analysis.db,
        &token,
        position_offset,
        &analysis.get_emmyrc(),
    ) {
        return Some(response);
    }

    // 3. Semantic declarations (decl / member / typedef).
    let decl = model
        .find_decl(token.clone().into())
        .or_else(|| find_index_metamethod_definition(&model, &token))?;
    let decl = prefer_same_file_global(&model, &token).unwrap_or(decl);
    goto_def_definition::goto_def_definition(&model, &analysis.db, &decl, &token)
}

pub struct DefinitionCapabilities;

impl RegisterCapabilities for DefinitionCapabilities {
    fn register_capabilities(server_capabilities: &mut ServerCapabilities, _: &ClientCapabilities) {
        server_capabilities.definition_provider = Some(OneOf::Left(true));
    }
}

/// Navigation preference only: inference still resolves globals across the workspace.
fn prefer_same_file_global(
    model: &emmylua_code_analysis::SemanticModel,
    token: &emmylua_parser::LuaSyntaxToken,
) -> Option<emmylua_code_analysis::SemanticId> {
    use emmylua_code_analysis::{DeclKind, SemanticId};
    use emmylua_parser::{LuaAssignStat, LuaCallExpr, LuaNameExpr};

    let name = LuaNameExpr::cast(token.parent()?)?;
    if name.get_parent::<LuaCallExpr>().is_some() {
        return None;
    }
    let id = model.find_decl(token.clone().into())?;
    let SemanticId::Decl(key) = &id else {
        return None;
    };
    let original = model.file_facts_of(key.file_id)?.decl_by_id(&id)?;
    if original.kind != DeclKind::Global {
        return None;
    }
    let root = model.chunk()?;
    let block = root.get_block()?;
    model
        .decls()?
        .iter()
        .filter(|decl| decl.kind == DeclKind::Global && decl.name == original.name)
        .filter_map(|decl| {
            let assignment =
                LuaAssignStat::cast(decl.owner_syntax?.to_node_from_root(root.syntax())?)?;
            if assignment.syntax().parent().as_ref() != Some(block.syntax())
                || assignment.get_range().end() > name.get_position()
            {
                return None;
            }
            Some(decl)
        })
        .max_by_key(|decl| decl.name_range.start())
        .map(|decl| decl.id.clone())
}

fn find_index_metamethod_definition(
    model: &SemanticModel,
    token: &LuaSyntaxToken,
) -> Option<SemanticId> {
    let origin = super::common::find_index_metamethod_origin(model, token)?;
    super::common::resolve_alias_origin(model, &origin).or(Some(origin))
}
