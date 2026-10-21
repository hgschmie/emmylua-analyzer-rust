use emmylua_code_analysis::{SemanticId, SemanticModel};
use emmylua_parser::{LuaAstNode, LuaIndexExpr, LuaIndexKey, LuaSyntaxToken, LuaTokenKind};

/// Presentation-only origin: do not change the identity used by rename/references.
pub(crate) fn find_index_metamethod_origin(
    model: &SemanticModel,
    token: &LuaSyntaxToken,
) -> Option<SemanticId> {
    let index = token.parent_ancestors().find_map(LuaIndexExpr::cast)?;
    // Prefix expressions and computed-key variables keep their own semantics.
    let key = index.get_index_key()?;
    // Definition token selection chooses '[' at the start of an integer key.
    let integer_boundary = token.kind() == LuaTokenKind::TkLeftBracket.into()
        && matches!(key, LuaIndexKey::Integer(_));
    if !integer_boundary && !key.get_range()?.contains_range(token.text_range()) {
        return None;
    }
    model.get_index_metamethod_origin(index)
}
