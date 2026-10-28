//! Separate read/write contracts for transformed properties.
use super::{
    SemanticModel,
    member::{self, MemberInfo},
};
use crate::{LuaMemberKey, LuaType, MemberAccess, SemanticId};
use emmylua_parser::{LuaAstNode, LuaIndexExpr, LuaIndexKey};

impl SemanticModel<'_> {
    /// Return the storage type checked by assignment.
    /// Expression-local assertions do not redefine a directional setter contract.
    pub fn assignment_target_type(&self, expr: emmylua_parser::LuaExpr) -> LuaType {
        if let emmylua_parser::LuaExpr::IndexExpr(index) = &expr
            && let Some((_, write)) = self.directional_index(index)
        {
            return write.typ;
        }
        self.type_of_expr(expr.get_syntax_id())
    }

    pub fn directional_declaration(&self, id: &SemanticId) -> Option<(MemberInfo, MemberInfo)> {
        let SemanticId::Member(key) = id else {
            return None;
        };
        let facts = self.file_facts_of(key.file_id)?;
        let member = facts.member_by_id(id)?;
        let def = facts.type_def_by_id(&member.owner)?;
        self.directional_member(&self.type_def_ref(def), &member.key)
    }

    /// Return the read type and the union of setter overloads, with generic substitutions.
    /// The write identity remains the first setter declaration for navigation.
    pub fn directional_member(
        &self,
        prefix: &LuaType,
        key: &LuaMemberKey,
    ) -> Option<(MemberInfo, MemberInfo)> {
        let (read, writes) = self.directional_member_views(prefix, key)?;
        let mut write = writes.first()?.clone();
        // A setter overload accepts a value matching any declared write contract.
        // Keep the first declaration identity for navigation, but expose every type.
        write.typ = LuaType::from_vec(writes.into_iter().map(|info| info.typ).collect());
        Some((read, write))
    }

    /// Preserve every setter declaration for contextual callback signature selection.
    pub(super) fn directional_member_views(
        &self,
        prefix: &LuaType,
        key: &LuaMemberKey,
    ) -> Option<(MemberInfo, Vec<MemberInfo>)> {
        let read = self.member_info(prefix, key)?;
        let SemanticId::Member(id) = read.id.as_ref()? else {
            return None;
        };
        let facts = self.file_facts_of(id.file_id)?;
        let field = facts.member_by_id(read.id.as_ref()?)?;
        if field.access == MemberAccess::ReadWrite {
            return None;
        }
        let mut writers = facts
            .members_with_access(field, MemberAccess::Write)
            .collect::<Vec<_>>();
        if writers.is_empty() {
            writers.push(field);
        }
        let writes = writers
            .into_iter()
            .map(|writer| member::member_info_for_declaration(self, prefix, key, &writer.id))
            .collect::<Option<Vec<_>>>()?;
        Some((read, writes))
    }

    pub fn directional_index(&self, index: &LuaIndexExpr) -> Option<(MemberInfo, MemberInfo)> {
        let prefix = self.type_of_expr(index.get_prefix_expr()?.get_syntax_id());
        let key = match index.get_index_key()? {
            LuaIndexKey::Name(n) => LuaMemberKey::Name(n.get_name_text().into()),
            LuaIndexKey::String(s) => LuaMemberKey::Name(s.get_value().into()),
            LuaIndexKey::Integer(n) => match n.get_number_value() {
                emmylua_parser::NumberResult::Int(n) => LuaMemberKey::Integer(n),
                _ => return None,
            },
            _ => return None,
        };
        self.directional_member(&prefix, &key)
    }
}
