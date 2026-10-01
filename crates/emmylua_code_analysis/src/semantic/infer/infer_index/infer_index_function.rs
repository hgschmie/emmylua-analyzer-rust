use emmylua_parser::{
    BinaryOperator, LuaAssignStat, LuaAstNode, LuaClosureExpr, LuaCommentOwner, LuaDocTag, LuaExpr,
    LuaIndexExpr, LuaIndexKey, LuaIndexMemberExpr, LuaLiteralToken, LuaStat, LuaSyntaxId,
    LuaSyntaxKind, LuaTableExpr, LuaTableField,
};

use crate::{
    DbIndex, InFiled, InferFailReason, InferGuard, LuaDeclId, LuaInferCache, LuaMemberKey,
    LuaOperator, LuaType, SignatureReturnStatus,
};

use super::{
    MemberLookupQuery, infer_expr, infer_index_key_type, infer_member_by_lookup,
    infer_member_by_operator_key_type,
};
use crate::semantic::infer::infer_binary::infer_binary_expr_result;
use hashbrown::HashSet;
use rowan::TextRange;

// Only specialize small, expression-only metamethods. A separate cache prevents
// one requested key's answer from leaking into another lookup or the function's
// ordinary inference. Carry the caller's depth across files to bound cycles.
const MAX_SPECIALIZATION_DEPTH: u32 = 32;
const MAX_EXPRESSION_NODES: usize = 64;

pub(super) fn infer_index_function(
    db: &DbIndex,
    cache: &LuaInferCache,
    operator: &LuaOperator,
    key_type: &LuaType,
) -> Option<LuaType> {
    if cache.infer_depth >= MAX_SPECIALIZATION_DEPTH
        || !matches!(
            key_type,
            LuaType::StringConst(_)
                | LuaType::DocStringConst(_)
                | LuaType::IntegerConst(_)
                | LuaType::DocIntegerConst(_)
        )
    {
        return None;
    }
    let LuaType::Signature(id) = operator.get_operator_func(db) else {
        return None;
    };
    let signature = db.get_signature_index().get(&id)?;
    if (signature.resolve_return == SignatureReturnStatus::DocResolve
        && !signature.get_return_type().is_any())
        || signature.is_generic()
        || !signature.overloads.is_empty()
    {
        return None;
    }
    let file_id = id.get_file_id();
    let root = db.get_vfs().get_syntax_tree(&file_id)?.get_red_root();
    let closure = root
        .token_at_offset(id.get_position())
        .right_biased()?
        .parent_ancestors()
        .find_map(LuaClosureExpr::cast)?;
    // The standard library supplies a contextual `fun(t, k): any` signature
    // for __index. That default must not hide a more precise body result, but
    // explicit annotations on the field still take precedence (including any).
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
    let key_param = closure.get_params_list()?.get_params().nth(1)?;
    let key_decl = LuaDeclId::new(file_id, key_param.get_position());
    let references = db.get_reference_index().get_local_reference(&file_id)?;
    if references.get_decl_references(&key_decl)?.mutable {
        return None;
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
    let mut local_cache = cache.fork_for_file(file_id);
    local_cache.infer_depth = cache.infer_depth + 1;
    IndexFunctionInference {
        db,
        cache: &mut local_cache,
        key_decl,
        key_type,
        remaining_nodes: MAX_EXPRESSION_NODES,
        closed_tables: HashSet::new(),
    }
    .infer(expr)
}

struct IndexFunctionInference<'a> {
    db: &'a DbIndex,
    cache: &'a mut LuaInferCache,
    key_decl: LuaDeclId,
    key_type: &'a LuaType,
    remaining_nodes: usize,
    closed_tables: HashSet<InFiled<TextRange>>,
}

impl IndexFunctionInference<'_> {
    fn infer(&mut self, expr: LuaExpr) -> Option<LuaType> {
        self.remaining_nodes = self.remaining_nodes.checked_sub(1)?;
        match expr {
            LuaExpr::ParenExpr(paren) => self.infer(paren.get_expr()?),
            LuaExpr::NameExpr(ref name) => {
                let references = self
                    .db
                    .get_reference_index()
                    .get_local_reference(&self.cache.get_file_id())?;
                let decl_id = references.get_decl_id(&name.get_range())?;
                if decl_id == self.key_decl {
                    return Some(self.key_type.clone());
                }
                // Captures can change after the closure is defined. Do not use
                // the declaration's original type for reassigned bindings.
                let decl = self.db.get_decl_index().get_decl(&decl_id)?;
                if !decl.is_local()
                    || decl.is_param()
                    || references.get_decl_references(&decl_id)?.mutable
                {
                    return None;
                }
                let typ = self
                    .cache
                    .with_no_flow(|cache| infer_expr(self.db, cache, expr))
                    .ok()?;
                if let LuaType::TableConst(table) = &typ
                    && !self.closed_tables.contains(table)
                    && !is_closed_local_table(self.db, decl_id).unwrap_or(false)
                {
                    return None;
                }
                if let LuaType::TableConst(table) = &typ {
                    self.closed_tables.insert(table.clone());
                }
                Some(typ)
            }
            LuaExpr::LiteralExpr(_) | LuaExpr::ClosureExpr(_) => {
                infer_expr(self.db, self.cache, expr).ok()
            }
            LuaExpr::BinaryExpr(binary) => {
                let op = binary.get_op_token()?.get_op();
                if !matches!(op, BinaryOperator::OpAnd | BinaryOperator::OpOr) {
                    return None;
                }
                let (left, right) = binary.get_exprs()?;
                let left_type = self.infer(left)?;
                if (op == BinaryOperator::OpAnd && left_type.is_always_falsy())
                    || (op == BinaryOperator::OpOr && left_type.is_always_truthy())
                {
                    return Some(left_type);
                }
                let right_type = self.infer(right)?;
                infer_binary_expr_result(self.db, binary, left_type, right_type).ok()
            }
            LuaExpr::IndexExpr(index) => {
                let prefix_type = self.infer(index.get_prefix_expr()?)?;
                let key = index.get_index_key()?;
                let key_type = match key {
                    LuaIndexKey::Expr(expr) => self.infer(expr)?,
                    _ => infer_index_key_type(self.db, self.cache, &key).ok()??,
                };
                let lookup = MemberLookupQuery::from_key_type(
                    LuaIndexMemberExpr::IndexExpr(index),
                    key_type.clone(),
                );
                let result = infer_member_by_lookup(
                    self.db,
                    self.cache,
                    &prefix_type,
                    &lookup,
                    &InferGuard::new(),
                );
                match result {
                    Ok(typ) => Some(typ),
                    Err(InferFailReason::FieldNotFound) => {
                        if let LuaType::TableConst(table) = &prefix_type
                            && self.closed_tables.contains(table)
                            && self.db.get_metatable_index().get(table).is_none()
                        {
                            return matches!(
                                lookup.key,
                                LuaMemberKey::Name(_) | LuaMemberKey::Integer(_)
                            )
                            .then_some(LuaType::Nil);
                        }
                        infer_member_by_operator_key_type(
                            self.db,
                            self.cache,
                            &prefix_type,
                            &key_type,
                            &InferGuard::new(),
                        )
                        .ok()
                    }
                    Err(_) => None,
                }
            }
            // Calls, branches and statement effects need interprocedural flow
            // analysis; retain the existing signature result for those bodies.
            _ => None,
        }
    }
}

// Missing members are only provably nil for a local literal whose uses cannot
// introduce untracked keys. Unknown-key writes may never reach the member index,
// so inspect references as well. Aliases, escapes and method calls fall back to
// the ordinary signature rather than pretending that the table is closed.
fn is_closed_local_table(db: &DbIndex, decl_id: LuaDeclId) -> Option<bool> {
    let decl = db.get_decl_index().get_decl(&decl_id)?;
    let tree = db.get_vfs().get_syntax_tree(&decl_id.file_id)?;
    let table = LuaTableExpr::cast(decl.get_value_syntax_id()?.to_node(tree)?)?;
    for (field, key) in table.get_fields_with_keys() {
        if matches!(key, LuaIndexKey::Expr(_)) {
            return Some(false);
        }
        if matches!(key, LuaIndexKey::Idx(_)) {
            match field.get_value_expr()? {
                LuaExpr::CallExpr(_) => return Some(false),
                LuaExpr::LiteralExpr(literal)
                    if matches!(literal.get_literal()?, LuaLiteralToken::Dots(_)) =>
                {
                    return Some(false);
                }
                _ => {}
            }
        }
    }
    let references = db
        .get_reference_index()
        .get_local_reference(&decl_id.file_id)?;
    for reference in &references.get_decl_references(&decl_id)?.cells {
        let node =
            LuaSyntaxId::new(LuaSyntaxKind::NameExpr.into(), reference.range).to_node(tree)?;
        let index = node.parent().and_then(LuaIndexExpr::cast)?;
        if index.get_prefix_expr()?.get_range() != reference.range
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
