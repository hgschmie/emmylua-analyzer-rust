//! Condition facts for stable computed accesses, which have no static member ID.

use emmylua_parser::{
    BinaryOperator, LuaAstNode, LuaExpr, LuaIndexExpr, LuaIndexKey, LuaLiteralToken, LuaStat,
    LuaSyntaxKind, LuaSyntaxNode, LuaVarExpr, UnaryOperator,
};
use rowan::TextSize;

use super::SemanticModel;
use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaClosureId};
use crate::{LuaMemberKey, LuaType};

#[derive(PartialEq, Eq)]
enum Part {
    Member(LuaMemberKey),
    Key(Box<Access>),
}

#[derive(PartialEq, Eq)]
struct Access {
    root: SemanticId,
    parts: Vec<Part>,
}

impl Access {
    fn depends_on(&self, decl: &SemanticId) -> bool {
        &self.root == decl
            || self
                .parts
                .iter()
                .any(|part| matches!(part, Part::Key(key) if key.depends_on(decl)))
    }
}

pub(super) fn guarded_type(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    offset: TextSize,
) -> Option<LuaType> {
    // Static members already have assignment-aware flow; do not replace their
    // more precise value with the collection's declared element contract.
    if model
        .resolve_member(index)
        .is_some_and(|member| member.member_id.is_some())
    {
        return None;
    }
    let access = access(model, &LuaExpr::IndexExpr(index.clone()))?;
    let prefix = index.get_prefix_expr()?;
    let owner_type = model.type_of_expr_at(prefix.get_syntax_id(), offset);
    // Dictionary contracts can be inherited or aliased. Explicit __index handlers
    // may return a different value on each read, so they remain excluded.
    match &owner_type {
        LuaType::Array(_) | LuaType::TableGeneric(_) => {}
        LuaType::Generic(generic)
            if generic.get_base_type_id() == crate::LuaTypeDeclId::global("table")
                && model
                    .type_def_of(&generic.get_base_type_id())
                    .is_some_and(|def| model.is_std_file(def.file_id)) => {}
        _ if !crate::semantic_model::inherited_index::dictionaries(model, &owner_type)
            .is_empty()
            && !model
                .member_infos(&owner_type)
                .iter()
                .any(|member| member.key == LuaMemberKey::Name("__index".into())) => {}
        _ => return None,
    }
    let base = model.type_of_expr(index.get_syntax_id());
    if !base.is_nullable() {
        return None;
    }
    if contains_call(index.ancestors::<LuaStat>().next()?.syntax(), offset) {
        return None;
    }
    let closure = LuaClosureId::from_node(index.syntax());
    let tree = model.flow_tree()?;
    let chunk = model.chunk()?;
    let mut current = tree.get_flow_id_at(offset)?;
    for _ in 0..128 {
        let node = tree.get_flow_node(current)?;
        match &node.kind {
            FlowNodeKind::TrueCondition(ptr) | FlowNodeKind::FalseCondition(ptr) => {
                let condition = ptr.to_node(&chunk)?;
                if LuaClosureId::from_node(condition.syntax()) != closure
                    || contains_call(condition.syntax(), offset)
                {
                    return None;
                }
                if condition.get_range().end() <= offset
                    && excludes_nil(
                        model,
                        &condition,
                        &access,
                        matches!(node.kind, FlowNodeKind::TrueCondition(_)),
                    )
                {
                    return Some(super::remove_type(base, &LuaType::Nil));
                }
            }
            FlowNodeKind::Assignment(ptr) => {
                let assignment = ptr.to_node(&chunk)?;
                if assignment.get_range().end() <= offset {
                    if LuaClosureId::from_node(assignment.syntax()) != closure {
                        return None;
                    }
                    let (vars, _) = assignment.get_var_and_expr_list();
                    if vars.iter().any(|var| match var {
                        LuaVarExpr::IndexExpr(_) => true,
                        LuaVarExpr::NameExpr(name) => {
                            name.get_name_text().as_deref() == Some("_ENV")
                                || reference(model, name)
                                    .is_some_and(|decl| access.depends_on(&decl))
                        }
                    }) {
                        return None;
                    }
                }
            }
            FlowNodeKind::CallExprStat(ptr) => {
                if contains_call(ptr.to_node(&chunk)?.syntax(), offset) {
                    return None;
                }
            }
            FlowNodeKind::LoopLabel | FlowNodeKind::NamedLabel(_) | FlowNodeKind::ImplFunc(_) => {
                return None;
            }
            _ => {}
        }
        for effect in tree.get_flow_effects(current) {
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                if value_syntax.get_range().end() <= offset && access.depends_on(decl) {
                    return None;
                }
                let value = value_syntax.to_node_from_root(chunk.syntax())?;
                if contains_call(&value, offset) {
                    return None;
                }
            }
        }
        let Some(FlowAntecedent::Single(next)) = node.antecedent else {
            return None;
        };
        current = next;
    }
    None
}

// Globals use their binding name rather than a particular declaration site.
// Locals retain declaration identity, keeping shadowed names separate.
fn reference(model: &SemanticModel, name: &emmylua_parser::LuaNameExpr) -> Option<SemanticId> {
    if let Some(decl) = model.resolve_name(name.get_position())
        && model
            .file_facts()?
            .decl_by_id(&decl)
            .is_some_and(|decl| decl.kind.is_local())
    {
        return Some(decl);
    }
    // A custom lexical environment can change the meaning of a global name.
    if model
        .file_facts()?
        .find_visible_decl_before_offset("_ENV", name.get_position())
        .is_some()
    {
        return None;
    }
    Some(SemanticId::name(name.get_name_text()?.into()))
}

fn access(model: &SemanticModel, expr: &LuaExpr) -> Option<Access> {
    match expr {
        LuaExpr::NameExpr(name) => Some(Access {
            root: reference(model, name)?,
            parts: Vec::new(),
        }),
        LuaExpr::IndexExpr(index) => {
            let mut result = access(model, &index.get_prefix_expr()?)?;
            let part = match index.get_index_key()? {
                LuaIndexKey::Name(name) => {
                    Part::Member(LuaMemberKey::Name(name.get_name_text().into()))
                }
                LuaIndexKey::String(string) => {
                    Part::Member(LuaMemberKey::Name(string.get_value().into()))
                }
                LuaIndexKey::Integer(number) => match number.get_number_value() {
                    emmylua_parser::NumberResult::Int(value) => {
                        Part::Member(LuaMemberKey::Integer(value))
                    }
                    _ => return None,
                },
                LuaIndexKey::Expr(expr) => Part::Key(Box::new(access(model, &expr)?)),
                _ => return None,
            };
            result.parts.push(part);
            Some(result)
        }
        LuaExpr::ParenExpr(paren) => access(model, &paren.get_expr()?),
        _ => None,
    }
}

fn excludes_nil(model: &SemanticModel, expr: &LuaExpr, expected: &Access, truthy: bool) -> bool {
    if access(model, expr).as_ref() == Some(expected) {
        return truthy;
    }
    match expr {
        LuaExpr::ParenExpr(paren) => paren
            .get_expr()
            .is_some_and(|inner| excludes_nil(model, &inner, expected, truthy)),
        LuaExpr::UnaryExpr(unary)
            if unary
                .get_op_token()
                .is_some_and(|op| op.get_op() == UnaryOperator::OpNot) =>
        {
            unary
                .get_expr()
                .is_some_and(|inner| excludes_nil(model, &inner, expected, !truthy))
        }
        LuaExpr::BinaryExpr(binary) => {
            let Some((left, right)) = binary.get_exprs() else {
                return false;
            };
            let Some(op) = binary.get_op_token() else {
                return false;
            };
            let is_nil = |expr: &LuaExpr| matches!(expr, LuaExpr::LiteralExpr(lit) if matches!(lit.get_literal(), Some(LuaLiteralToken::Nil(_))));
            match op.get_op() {
                BinaryOperator::OpEq | BinaryOperator::OpNe => {
                    let non_nil = (op.get_op() == BinaryOperator::OpNe) == truthy;
                    non_nil
                        && ((is_nil(&left) && access(model, &right).as_ref() == Some(expected))
                            || (is_nil(&right) && access(model, &left).as_ref() == Some(expected)))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn contains_call(node: &LuaSyntaxNode, before: TextSize) -> bool {
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        match node.kind().into() {
            LuaSyntaxKind::ClosureExpr => {}
            LuaSyntaxKind::CallExpr if node.text_range().end() <= before => return true,
            _ => pending.extend(node.children()),
        }
    }
    false
}
