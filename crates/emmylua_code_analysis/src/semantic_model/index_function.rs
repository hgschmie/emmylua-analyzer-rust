//! Bounded, key-specific inference for expression-only __index functions.
use super::{
    SemanticModel,
    member::{MemberInfo, table_metatable_type},
};
use crate::{DeclKind, InFiled, LuaMemberKey, LuaType, SemanticId};
use emmylua_parser::{
    BinaryOperator, LuaAssignStat, LuaAstNode, LuaClosureExpr, LuaCommentOwner, LuaDocTag, LuaExpr,
    LuaFuncStat, LuaIndexExpr, LuaIndexKey, LuaLiteralToken, LuaStat, LuaSyntaxId, LuaTableExpr,
    LuaTableField, LuaVarExpr,
};
use hashbrown::HashSet;
use std::cell::Cell;

thread_local! { static DEPTH: Cell<u32> = const { Cell::new(0) }; }
struct DepthGuard;
impl Drop for DepthGuard {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
    }
}

pub(crate) fn infer(
    model: &SemanticModel,
    info: &MemberInfo,
    key: &LuaMemberKey,
) -> Option<LuaType> {
    infer_detail(model, info, key, false).map(|result| result.0)
}

fn infer_detail(
    model: &SemanticModel,
    info: &MemberInfo,
    key: &LuaMemberKey,
    origin: bool,
) -> Option<IndexResult> {
    let key_type = match key {
        LuaMemberKey::Name(name) => LuaType::StringConst(name.clone().into()),
        LuaMemberKey::Integer(value) => LuaType::IntegerConst(*value),
        _ => return None,
    };
    if !DEPTH.with(|d| {
        if d.get() >= 32 {
            false
        } else {
            d.set(d.get() + 1);
            true
        }
    }) {
        return None;
    }
    let _depth = DepthGuard;
    let id = info.id.as_ref()?;
    let file = info.file_id?;
    let model = model.model_for(file);
    let facts = model.file_facts()?;
    let member = facts.member_by_id(id)?;
    let root = model.chunk()?;
    let closure = LuaClosureExpr::cast(member.value_syntax?.to_node_from_root(root.syntax())?)?;
    let signature = facts.signature_by_closure(closure.get_syntax_id())?;
    if signature
        .docs
        .as_ref()
        .is_some_and(|docs| !docs.generic_params.is_empty() || !docs.overloads.is_empty())
    {
        return None;
    }
    let field = closure.get_parent::<LuaTableField>()?;
    if field.get_comments().iter().any(|comment| {
        comment.descendants::<LuaDocTag>().any(|tag| {
            matches!(
                tag,
                LuaDocTag::Return(_)
                    | LuaDocTag::Type(_)
                    | LuaDocTag::Overload(_)
                    | LuaDocTag::ReturnOverload(_)
                    | LuaDocTag::ReturnCast(_)
            )
        })
    }) {
        return None;
    }
    let param = closure.get_params_list()?.get_params().nth(1)?;
    let key_decl = model.decl_by_offset(param.get_position())?;
    if binding_is_mutated(&model, &key_decl) {
        return None;
    }
    if let Some(param_type) = model.param_type(closure.get_syntax_id(), 1)
        && !matches!(param_type, LuaType::Unknown | LuaType::Any)
        && !super::type_check::is_compatible(&model, &param_type, &key_type)
    {
        return Some(IndexResult(LuaType::Nil, None, None));
    }
    let block = closure.get_block()?;
    let mut stats = block.get_stats();
    let LuaStat::ReturnStat(ret) = stats.next()? else {
        return None;
    };
    if stats.next().is_some() {
        return None;
    }
    let mut exprs = ret.get_expr_list();
    let expr = exprs.next()?;
    if exprs.next().is_some() {
        return None;
    }
    let mut inference = Inference {
        model: &model,
        key_decl,
        key_type,
        remaining: 64,
        closed: HashSet::new(),
    };
    let typ = inference.infer(expr.clone())?;
    let result_closure = if origin {
        inference.result_closure(expr.clone())
    } else {
        None
    };
    let origin = if origin && !typ.is_nil() {
        inference.find_origin(expr)
    } else {
        None
    };
    Some(IndexResult(
        typ,
        origin,
        result_closure.map(|id| InFiled::new(file, id)),
    ))
}

#[derive(PartialEq)]
struct IndexResult(LuaType, Option<SemanticId>, Option<InFiled<LuaSyntaxId>>);

struct Inference<'a, 'db> {
    model: &'a SemanticModel<'db>,
    key_decl: SemanticId,
    key_type: LuaType,
    remaining: usize,
    closed: HashSet<InFiled<rowan::TextRange>>,
}
impl Inference<'_, '_> {
    fn result_closure(&mut self, expr: LuaExpr) -> Option<LuaSyntaxId> {
        self.remaining = self.remaining.checked_sub(1)?;
        match expr {
            LuaExpr::ClosureExpr(closure) => Some(closure.get_syntax_id()),
            LuaExpr::ParenExpr(paren) => self.result_closure(paren.get_expr()?),
            LuaExpr::BinaryExpr(binary) => {
                let (left, right) = binary.get_exprs()?;
                let typ = self.infer(left.clone())?;
                let selected = match binary.get_op_token()?.get_op() {
                    BinaryOperator::OpAnd if typ.is_always_truthy() => right,
                    BinaryOperator::OpAnd if typ.is_always_falsy() => left,
                    BinaryOperator::OpOr if typ.is_always_truthy() => left,
                    BinaryOperator::OpOr if typ.is_always_falsy() => right,
                    _ => return None,
                };
                self.result_closure(selected)
            }
            _ => None,
        }
    }

    // Definition-only provenance: do not make synthetic members part of the
    // normal semantic identity used by rename, references and diagnostics.
    fn find_origin(&mut self, expr: LuaExpr) -> Option<SemanticId> {
        self.remaining = self.remaining.checked_sub(1)?;
        match expr {
            LuaExpr::ParenExpr(paren) => self.find_origin(paren.get_expr()?),
            LuaExpr::IndexExpr(index) => {
                let prefix = index.get_prefix_expr()?;
                let LuaType::TableConst(table) = self.infer(prefix.clone())? else {
                    return None;
                };
                let key = self.index_key(index.get_index_key()?)?;
                if !self.has_unique_definition(prefix, &key)? {
                    return None;
                }
                self.model
                    .member_info(&LuaType::TableConst(table), &key)?
                    .id
            }
            LuaExpr::BinaryExpr(binary) => {
                let (left, right) = binary.get_exprs()?;
                let left_type = self.infer(left.clone())?;
                let selected = match binary.get_op_token()?.get_op() {
                    BinaryOperator::OpAnd if left_type.is_always_truthy() => right,
                    BinaryOperator::OpAnd if left_type.is_always_falsy() => left,
                    BinaryOperator::OpOr if left_type.is_always_truthy() => left,
                    BinaryOperator::OpOr if left_type.is_always_falsy() => right,
                    _ => return None,
                };
                self.find_origin(selected)
            }
            LuaExpr::ClosureExpr(closure) => {
                // Follow a forwarding wrapper's callee, not merely the table
                // lookup used as its existence guard. Parameter identities are
                // resolved by infer(), so a shadowed key cannot be substituted.
                let block = closure.get_block()?;
                let mut stats = block.get_stats();
                let LuaStat::ReturnStat(ret) = stats.next()? else {
                    return None;
                };
                if stats.next().is_some() {
                    return None;
                }
                let mut exprs = ret.get_expr_list();
                let LuaExpr::CallExpr(call) = exprs.next()? else {
                    return None;
                };
                if exprs.next().is_some() {
                    return None;
                }
                self.find_origin(call.get_prefix_expr()?)
            }
            _ => None,
        }
    }

    fn has_unique_definition(&mut self, prefix: LuaExpr, key: &LuaMemberKey) -> Option<bool> {
        if let LuaExpr::ParenExpr(paren) = prefix {
            return self.has_unique_definition(paren.get_expr()?, key);
        }
        let LuaExpr::NameExpr(name) = prefix else {
            return None;
        };
        let facts = self.model.file_facts()?;
        let decl_id = self.model.resolve_name(name.get_position())?;
        let decl = facts.decl_by_id(&decl_id)?;
        let root = self.model.chunk()?;
        let table = LuaTableExpr::cast(decl.value_expr_syntax?.to_node_from_root(root.syntax())?)?;
        let mut definitions = 0;
        for (_, key_expr) in table.get_fields_with_keys() {
            if self.index_key(key_expr)? == *key {
                definitions += 1;
            }
        }
        for reference in &facts.name_uses {
            if self
                .model
                .resolve_name(reference.syntax.get_range().start())
                .as_ref()
                != Some(&decl_id)
            {
                continue;
            }
            let node = reference.syntax.to_node_from_root(root.syntax())?;
            let index = node.parent().and_then(LuaIndexExpr::cast)?;
            let is_write = index.get_parent::<LuaFuncStat>().is_some()
                || index.get_parent::<LuaAssignStat>().is_some_and(|assign| {
                    assign
                        .get_var_and_expr_list()
                        .0
                        .iter()
                        .any(|var| var.get_range() == index.get_range())
                });
            if is_write && self.index_key(index.get_index_key()?)? == *key {
                definitions += 1;
            }
        }
        Some(definitions == 1)
    }

    fn index_key(&mut self, key: LuaIndexKey) -> Option<LuaMemberKey> {
        Some(match key {
            LuaIndexKey::Name(name) => LuaMemberKey::Name(name.get_name_text().into()),
            LuaIndexKey::String(s) => LuaMemberKey::Name(s.get_value().into()),
            LuaIndexKey::Integer(n) => match n.get_number_value() {
                emmylua_parser::NumberResult::Int(n) => LuaMemberKey::Integer(n),
                _ => return None,
            },
            LuaIndexKey::Expr(expr) => match self.infer(expr)? {
                LuaType::StringConst(s) | LuaType::DocStringConst(s) => {
                    LuaMemberKey::Name(s.as_str().into())
                }
                LuaType::IntegerConst(n) | LuaType::DocIntegerConst(n) => LuaMemberKey::Integer(n),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn infer(&mut self, expr: LuaExpr) -> Option<LuaType> {
        self.remaining = self.remaining.checked_sub(1)?;
        match expr {
            LuaExpr::ParenExpr(paren) => self.infer(paren.get_expr()?),
            LuaExpr::NameExpr(ref name) => {
                let id = self.model.resolve_name(name.get_position())?;
                if id == self.key_decl {
                    return Some(self.key_type.clone());
                }
                let decl = self.model.file_facts()?.decl_by_id(&id)?;
                if !matches!(decl.kind, DeclKind::Local { .. })
                    || binding_is_mutated(self.model, &id)
                {
                    return None;
                }
                let typ = self.model.type_of_decl(&id)?;
                if let LuaType::TableConst(table) = &typ {
                    if !self.closed.contains(table)
                        && !is_closed_local_table(self.model, &id).unwrap_or(false)
                    {
                        return None;
                    }
                    self.closed.insert(table.clone());
                }
                Some(typ)
            }
            LuaExpr::ClosureExpr(ref closure) => Some(
                self.model
                    .type_of_signature(closure.get_syntax_id())
                    .map(|fun| LuaType::DocFunction(fun.into()))
                    .unwrap_or(LuaType::Function),
            ),
            LuaExpr::LiteralExpr(_) => Some(self.model.type_of_expr(expr.get_syntax_id())),
            LuaExpr::BinaryExpr(binary) => {
                let op = binary.get_op_token()?.get_op();
                if !matches!(op, BinaryOperator::OpAnd | BinaryOperator::OpOr) {
                    return None;
                }
                let (left, right) = binary.get_exprs()?;
                let left = self.infer(left)?;
                if (op == BinaryOperator::OpAnd && left.is_always_falsy())
                    || (op == BinaryOperator::OpOr && left.is_always_truthy())
                {
                    return Some(left);
                }
                let right = self.infer(right)?;
                Some(super::infer::vm::binary_type(self.model, op, &left, &right))
            }
            LuaExpr::IndexExpr(index) => {
                let prefix = self.infer(index.get_prefix_expr()?)?;
                let key = self.index_key(index.get_index_key()?)?;
                if let Some(info) = self.model.member_info(&prefix, &key) {
                    return Some(info.typ);
                }
                if let LuaType::TableConst(table) = &prefix
                    && self.closed.contains(table)
                    && table_metatable_type(self.model, table).is_none()
                {
                    return Some(LuaType::Nil);
                }
                None
            }
            _ => None,
        }
    }
}

fn binding_is_mutated(model: &SemanticModel, id: &SemanticId) -> bool {
    let Some(root) = model.chunk() else {
        return true;
    };
    root.descendants::<LuaAssignStat>().any(|assign| assign.get_var_and_expr_list().0.into_iter().any(|var|
        matches!(var, LuaVarExpr::NameExpr(name) if model.resolve_name(name.get_position()).as_ref() == Some(id))))
}

fn is_closed_local_table(model: &SemanticModel, id: &SemanticId) -> Option<bool> {
    let facts = model.file_facts()?;
    let decl = facts.decl_by_id(id)?;
    let root = model.chunk()?;
    let table = LuaTableExpr::cast(decl.value_expr_syntax?.to_node_from_root(root.syntax())?)?;
    for (field, key) in table.get_fields_with_keys() {
        if matches!(key, LuaIndexKey::Expr(_)) {
            return Some(false);
        }
        if matches!(key, LuaIndexKey::Idx(_)) {
            match field.get_value_expr()? {
                LuaExpr::CallExpr(_) => return Some(false),
                LuaExpr::LiteralExpr(lit)
                    if matches!(lit.get_literal()?, LuaLiteralToken::Dots(_)) =>
                {
                    return Some(false);
                }
                _ => {}
            }
        }
    }
    for reference in &facts.name_uses {
        if model
            .resolve_name(reference.syntax.get_range().start())
            .as_ref()
            != Some(id)
        {
            continue;
        }
        let node = reference.syntax.to_node_from_root(root.syntax())?;
        let index = node.parent().and_then(LuaIndexExpr::cast)?;
        if index.get_prefix_expr()?.get_range() != reference.syntax.get_range()
            || index.get_index_token()?.is_colon()
        {
            return Some(false);
        }
        if let Some(assign) = index.get_parent::<LuaAssignStat>()
            && assign
                .get_var_and_expr_list()
                .0
                .iter()
                .any(|var| var.get_range() == index.get_range())
            && !matches!(
                index.get_index_key()?,
                LuaIndexKey::Name(_) | LuaIndexKey::String(_) | LuaIndexKey::Integer(_)
            )
        {
            return Some(false);
        }
    }
    Some(true)
}

// Ordinary signature inference also visits the body. Bound that fallback before
// asking it to process an oversized unannotated metamethod.
pub(crate) fn oversized(
    model: &SemanticModel,
    member: &crate::Member,
    file: crate::FileId,
) -> bool {
    let Some(tree) = model.syntax_tree_of(file) else {
        return false;
    };
    let Some(closure) = member
        .value_syntax
        .and_then(|syntax| syntax.to_node_from_root(&tree.get_red_root()))
        .and_then(LuaClosureExpr::cast)
    else {
        return false;
    };
    let Some(field) = closure.get_parent::<LuaTableField>() else {
        return false;
    };
    if field.get_comments().iter().any(|comment| {
        comment
            .descendants::<LuaDocTag>()
            .any(|tag| matches!(tag, LuaDocTag::Return(_) | LuaDocTag::Type(_)))
    }) {
        return false;
    }
    closure.descendants::<LuaExpr>().take(65).count() > 64
}

impl SemanticModel<'_> {
    /// Navigation-only provenance; never establishes identity for rename/references.
    pub fn get_index_metamethod_origin(&self, index: LuaIndexExpr) -> Option<SemanticId> {
        self.index_metamethod_result(index)?.1
    }

    /// Syntax of the selected wrapper, used only to present its own documentation.
    pub fn get_index_metamethod_wrapper(
        &self,
        index: LuaIndexExpr,
    ) -> Option<InFiled<LuaSyntaxId>> {
        self.index_metamethod_result(index)?.2
    }

    fn index_metamethod_result(&self, index: LuaIndexExpr) -> Option<IndexResult> {
        let key = match index.get_index_key()? {
            LuaIndexKey::Name(n) => LuaMemberKey::Name(n.get_name_text().into()),
            LuaIndexKey::String(s) => LuaMemberKey::Name(s.get_value().into()),
            LuaIndexKey::Integer(n) => match n.get_number_value() {
                emmylua_parser::NumberResult::Int(n) => LuaMemberKey::Integer(n),
                _ => return None,
            },
            _ => return None,
        };
        let typ = self.type_of_expr(index.get_prefix_expr()?.get_syntax_id());
        fn origin(model: &SemanticModel, typ: &LuaType, key: &LuaMemberKey) -> Option<IndexResult> {
            if model
                .member_info(typ, key)
                .is_some_and(|info| info.id.is_some())
            {
                return None;
            }
            match typ {
                LuaType::TableConst(table) => {
                    let info = super::member::table_metatable_index_info(model, table)?;
                    infer_detail(model, &info, key, true)
                }
                LuaType::Union(union) => {
                    let types = union.into_vec();
                    let mut origins = types.iter().filter_map(|typ| origin(model, typ, key));
                    let first = origins.next()?;
                    origins.all(|id| id == first).then_some(first)
                }
                _ => None,
            }
        }
        origin(self, &typ, &key)
    }
}
