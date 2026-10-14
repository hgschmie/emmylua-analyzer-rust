//! Separate read/write contracts for transformed properties.
use super::{SemanticModel, member::MemberInfo};
use crate::{LuaMemberKey, LuaType, MemberAccess, SemanticId};
use emmylua_parser::{LuaAstNode, LuaIndexExpr, LuaIndexKey};

impl SemanticModel<'_> {
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

    /// Return the declared read and write views, with generic substitutions.
    pub fn directional_member(
        &self,
        prefix: &LuaType,
        key: &LuaMemberKey,
    ) -> Option<(MemberInfo, MemberInfo)> {
        let read = self.member_info(prefix, key)?;
        let SemanticId::Member(id) = read.id.as_ref()? else {
            return None;
        };
        let facts = self.file_facts_of(id.file_id)?;
        let field = facts.member_by_id(read.id.as_ref()?)?;
        if field.access == MemberAccess::ReadWrite {
            return None;
        }
        let writer = facts
            .members
            .iter()
            .find(|other| {
                other.owner == field.owner
                    && other.key == field.key
                    && other.access == MemberAccess::Write
            })
            .unwrap_or(field);
        let mut typ = writer
            .value_syntax
            .map(|syntax| self.doc_type_lua_rich_in(id.file_id, syntax))
            .or_else(|| self.type_of_member(&writer.id))?;
        if writer.is_nullable && !typ.is_nullable() {
            typ = LuaType::from_vec(vec![typ, LuaType::Nil]);
        }
        if let LuaType::Generic(generic) = prefix
            && let SemanticId::TypeDef(owner) = &writer.owner
            && let Some(def) = facts.type_def_by_id(&SemanticId::TypeDef(owner.clone()))
        {
            let names = def
                .generic_params
                .iter()
                .map(|param| param.name.clone())
                .collect::<Vec<_>>();
            typ = self.substitute_generic_params_named(&typ, generic.get_params(), &names);
        }
        let write = MemberInfo {
            key: key.clone(),
            typ,
            id: Some(writer.id.clone()),
            file_id: Some(id.file_id),
            is_method: writer.is_method,
        };
        Some((read, write))
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
