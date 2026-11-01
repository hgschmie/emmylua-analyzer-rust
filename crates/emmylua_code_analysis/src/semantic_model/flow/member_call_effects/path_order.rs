//! Canonical receiver ordering for invocation-state equality, independent of Debug.

use super::{Access, SemanticId};
use crate::{FileId, semantic_db::def::TypeScope};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum RootKey<'a> {
    Decl(FileId, u32, u32),
    Member(FileId, u32, u32),
    TypeDef((u8, u32), &'a str),
    Signature(FileId, bool, u16, u32, u32),
    Name(&'a str),
}

fn root_key(id: &SemanticId) -> RootKey<'_> {
    match id {
        SemanticId::Decl(key) => RootKey::Decl(
            key.file_id,
            key.name_range.start().into(),
            key.name_range.end().into(),
        ),
        SemanticId::Member(key) => RootKey::Member(
            key.file_id,
            key.key_range.start().into(),
            key.key_range.end().into(),
        ),
        SemanticId::TypeDef(key) => {
            let scope = match key.scope {
                TypeScope::Global => (0, 0),
                TypeScope::Internal(workspace) => (1, workspace.id),
                TypeScope::File(file) => (2, file.id),
            };
            RootKey::TypeDef(scope, &key.full_name)
        }
        SemanticId::Signature(key) => {
            let syntax = key.closure_syntax;
            let kind = if syntax.is_token() {
                syntax.get_token_kind() as u16
            } else {
                syntax.get_kind() as u16
            };
            RootKey::Signature(
                key.file_id,
                syntax.is_token(),
                kind,
                syntax.get_range().start().into(),
                syntax.get_range().end().into(),
            )
        }
        SemanticId::Name(name) => RootKey::Name(name),
    }
}

pub(super) fn canonicalize(paths: &mut Vec<Access>) {
    paths.sort_by(|left, right| {
        root_key(&left.0)
            .cmp(&root_key(&right.0))
            .then_with(|| left.1.cmp(&right.1))
    });
    paths.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LuaMemberKey;
    use rowan::TextRange;

    #[test]
    fn receiver_order_is_independent_of_discovery_order() {
        let range = TextRange::new(1.into(), 2.into());
        let syntax = emmylua_parser::LuaSyntaxId::new(
            emmylua_parser::LuaSyntaxKind::ClosureExpr.into(),
            range,
        );
        let roots = [
            SemanticId::decl(FileId::new(1), range),
            SemanticId::decl(FileId::new(2), range),
            SemanticId::member(FileId::new(1), range),
            SemanticId::name("receiver".into()),
            SemanticId::type_def(TypeScope::Global, "Receiver".into()),
            SemanticId::type_def(TypeScope::File(FileId::new(1)), "Receiver".into()),
            SemanticId::type_def(
                TypeScope::Internal(crate::semantic_db::def::WorkspaceId::MAIN),
                "Receiver".into(),
            ),
            SemanticId::signature(FileId::new(1), syntax),
            SemanticId::signature(FileId::new(2), syntax),
        ];
        let mut paths: Vec<_> = roots
            .into_iter()
            .flat_map(|root| {
                [
                    (root.clone(), vec![]),
                    (root, vec![LuaMemberKey::Name("value".into())]),
                ]
            })
            .collect();
        let mut reversed = paths.iter().rev().cloned().collect();
        paths.extend(paths.clone());
        canonicalize(&mut paths);
        canonicalize(&mut reversed);
        assert_eq!(paths, reversed);
        assert_eq!(paths.len(), 18);
    }
}
