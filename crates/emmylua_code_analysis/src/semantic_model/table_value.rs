//! Supplied constructor values, or stable inferred fields of a table reference.
//!
//! Field flow affects field reads, not the structural type of the whole table.

use emmylua_parser::{
    LuaAstNode, LuaCallExpr, LuaExpr, LuaLocalStat, LuaSyntaxKind, LuaSyntaxNode, LuaTableExpr,
};
use hashbrown::HashSet;
use rowan::TextRange;

use super::SemanticModel;
use super::member::{
    MemberCollectMode, member_infos_all, member_infos_with_key_mode, table_metatable_index_info,
};
use super::table_constructor::{ConstructorValue, TableConstructor, member_key};
use super::type_check::TypeCheckFailReason;
use super::type_eval::expand_alias_branches;
use crate::{InFiled, LuaMemberKey, LuaType, SemanticId};

pub(crate) struct TableValue<'db> {
    model: SemanticModel<'db>,
    identity: InFiled<TextRange>,
    constructor: TableConstructor,
    is_constructor: bool,
    argument_only: bool,
}

impl<'db> TableValue<'db> {
    pub(crate) fn new(
        model: &SemanticModel<'db>,
        identity: &InFiled<TextRange>,
        expression: Option<&InFiled<TextRange>>,
    ) -> Result<Self, TypeCheckFailReason> {
        let model = model.model_for(identity.file_id);
        let table = model
            .syntax_tree()
            .and_then(|tree| {
                tree.get_red_root()
                    .covering_element(identity.value)
                    .into_node()
            })
            .and_then(LuaTableExpr::cast)
            .ok_or(TypeCheckFailReason::Unresolved)?;
        // A literal inside a call contributes to the call's result.
        let is_constructor = expression.is_some_and(|expr| {
            expr.file_id == identity.file_id
                && expr.value.contains_range(identity.value)
                && !table
                    .ancestors::<LuaCallExpr>()
                    .any(|call| expr.value.contains_range(call.get_range()))
        });
        let argument_only = !is_constructor && table_is_argument_only(&model, &table);
        let constructor = TableConstructor::new(&model, &table);
        Ok(Self {
            model,
            identity: identity.clone(),
            constructor,
            is_constructor,
            argument_only,
        })
    }

    pub(crate) fn is_constructor(&self) -> bool {
        self.is_constructor
    }

    fn own_member_value(
        &self,
        key: &LuaMemberKey,
    ) -> Result<Option<ConstructorValue<'_>>, TypeCheckFailReason> {
        let initial = self.constructor.member_value(key)?;
        if self.is_constructor {
            return Ok(initial);
        }
        // An unannotated string or boolean can distinguish union alternatives.
        // Keep its literal type when the enclosing table is used only as an argument.
        if self.argument_only
            && let Some(value) = &initial
            && matches!(
                value.typ,
                LuaType::StringConst(_) | LuaType::BooleanConst(_)
            )
            && let Some(expr) = value.expr
            && self.model.file_facts().is_some_and(|facts| {
                facts
                    .member_by_value_syntax(expr.get_syntax_id())
                    .is_none_or(|field| field.doc_type_syntax.is_none())
            })
        {
            return Ok(initial);
        }
        let has_initial = initial.as_ref().is_some_and(|value| !value.typ.is_nil());
        let owner = SemanticId::member(self.identity.file_id, self.identity.value);
        // Keep the constructor field's contract when it supplies a value.
        // Otherwise, use the assignment that adds the field to this table.
        let member = self.model.require_complete(|| {
            let facts = self.model.file_facts();
            member_infos_with_key_mode(
                &self.model,
                &LuaType::TableConst(self.identity.clone()),
                key,
                MemberCollectMode::AllDefinitions,
            )
            .into_iter()
            .find(|info| {
                info.id
                    .as_ref()
                    .and_then(|id| facts?.member_by_id(id))
                    .is_some_and(|member| {
                        if has_initial {
                            member.owner == owner
                        } else {
                            member.owner != owner
                        }
                    })
            })
        })?;
        let typ = if has_initial {
            initial.map(|value| value.typ)
        } else {
            member
                .as_ref()
                .map(|info| info.typ.clone())
                .or_else(|| initial.map(|value| value.typ))
        };
        let Some(typ) = typ else {
            return Ok(None);
        };
        let typ = self
            .model
            .member_contract_type(member.as_ref().and_then(|info| info.id.as_ref()), typ);
        // Resolve nullable aliases before deciding whether __index supplies a value.
        let typ = self
            .model
            .require_complete(|| expand_alias_branches(&self.model, &typ))?
            .ok_or(TypeCheckFailReason::Unresolved)?;
        Ok(Some(ConstructorValue { typ, expr: None }))
    }

    pub(crate) fn member_value(
        &self,
        key: &LuaMemberKey,
    ) -> Result<Option<ConstructorValue<'_>>, TypeCheckFailReason> {
        let actual = self.own_member_value(key)?;
        if self.is_constructor
            || actual
                .as_ref()
                .is_some_and(|value| !value.typ.is_nullable())
        {
            return Ok(actual);
        }
        // __index supplies reads of absent fields; it does not add dictionary entries.
        let mut types = Vec::new();
        if let Some(value) = &actual {
            push_present_types(&mut types, &value.typ);
        }
        let mut identity = self.identity.clone();
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(identity.clone()) {
                return Err(TypeCheckFailReason::Unresolved);
            }
            let model = self.model.model_for(identity.file_id);
            let index = model.require_complete(|| table_metatable_index_info(&model, &identity))?;
            match index.map(|info| info.typ) {
                Some(LuaType::TableConst(table)) => {
                    let prototype = Self::new(&self.model, &table, None)?;
                    let value = prototype.own_member_value(key)?;
                    if let Some(value) = value {
                        push_present_types(&mut types, &value.typ);
                        if !value.typ.is_nullable() {
                            break;
                        }
                    }
                    identity = table;
                }
                Some(LuaType::DocFunction(function)) => {
                    types.push(
                        function
                            .get_ret()
                            .get_result_slot_type(0)
                            .unwrap_or(LuaType::Nil),
                    );
                    break;
                }
                Some(index) => {
                    types.push(
                        model
                            .require_complete(|| model.member_type(&index, key))?
                            .unwrap_or(LuaType::Nil),
                    );
                    break;
                }
                None => {
                    if types.is_empty() {
                        return Ok(actual);
                    }
                    types.push(LuaType::Nil);
                    break;
                }
            }
        }
        Ok(Some(ConstructorValue {
            typ: LuaType::from_vec(types),
            expr: None,
        }))
    }

    pub(crate) fn field_entries(
        &self,
    ) -> impl Iterator<Item = Result<(LuaType, ConstructorValue<'_>), TypeCheckFailReason>> {
        let entries = self
            .constructor
            .field_entries()
            .flat_map(|entries| match entries {
                Ok(entries) => entries.into_iter().map(Ok).collect(),
                Err(err) => vec![Err(err)],
            });
        if self.is_constructor {
            return entries.collect::<Vec<_>>().into_iter();
        }
        let mut keys = HashSet::new();
        let mut values = Vec::new();
        for entry in entries {
            match entry {
                Ok((key, value)) => {
                    if let Some(member) = member_key(&key) {
                        keys.insert(member);
                    } else {
                        values.push(Ok((key, value)));
                    }
                }
                Err(err) => values.push(Err(err)),
            }
        }
        match self.model.require_complete(|| {
            member_infos_all(&self.model, &LuaType::TableConst(self.identity.clone()))
                .into_iter()
                .map(|info| info.key)
        }) {
            Ok(members) => keys.extend(members),
            Err(err) => values.push(Err(err)),
        }
        for key in keys {
            match self.own_member_value(&key) {
                Ok(Some(value)) if value.typ != LuaType::Nil => {
                    if let Some(key) = key.to_index_type() {
                        values.push(Ok((key, value)));
                    }
                }
                Ok(_) => {}
                Err(err) => values.push(Err(err)),
            }
        }
        values.into_iter()
    }
}

fn push_present_types(types: &mut Vec<LuaType>, typ: &LuaType) {
    match typ {
        LuaType::Union(union) => types.extend(union.iter().filter(|ty| !ty.is_nil()).cloned()),
        LuaType::Nil => {}
        typ => types.push(typ.clone()),
    }
}

/// Skip only grouping and containing constructors. Storing, indexing, returning
/// or aliasing the enclosing table all require its ordinary mutable contract.
fn enclosing_argument_value(mut node: LuaSyntaxNode) -> LuaSyntaxNode {
    while let Some(parent) = node.parent() {
        match parent.kind().into() {
            LuaSyntaxKind::ParenExpr
            | LuaSyntaxKind::TableFieldAssign
            | LuaSyntaxKind::TableFieldValue
            | LuaSyntaxKind::TableArrayExpr
            | LuaSyntaxKind::TableObjectExpr => node = parent,
            _ => break,
        }
    }
    node
}

fn table_is_argument_only(model: &SemanticModel, table: &LuaTableExpr) -> bool {
    let value = enclosing_argument_value(table.syntax().clone());
    let Some(parent) = value.parent() else {
        return false;
    };
    if parent.kind() == LuaSyntaxKind::CallArgList.into() {
        return true;
    }
    let Some(decl) = LuaLocalStat::cast(parent)
        .and_then(|local| local.get_local_name_by_value(LuaExpr::cast(value)?))
        .and_then(|name| model.file_facts()?.decl_at_offset(name.get_position()))
    else {
        return false;
    };
    let Some(tree) = model.syntax_tree() else {
        return false;
    };
    let root = tree.get_red_root();
    let references = model.decl_references(&decl.id);
    !references.is_empty()
        && references.iter().all(|syntax| {
            syntax
                .to_node_from_root(&root)
                .and_then(|node| enclosing_argument_value(node).parent())
                .is_some_and(|parent| parent.kind() == LuaSyntaxKind::CallArgList.into())
        })
}
