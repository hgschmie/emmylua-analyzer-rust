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
    Local(SemanticId),
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
                .any(|part| matches!(part, Part::Local(key) if key == decl))
    }
}

pub(super) fn guarded_type(
    model: &SemanticModel,
    index: &LuaIndexExpr,
    offset: TextSize,
) -> Option<LuaType> {
    let access = access(model, &LuaExpr::IndexExpr(index.clone()))?;
    if !access
        .parts
        .iter()
        .any(|part| matches!(part, Part::Local(_)))
    {
        return None;
    }
    let prefix = index.get_prefix_expr()?;
    let owner_type = model.type_of_expr_at(prefix.get_syntax_id(), offset);
    // Only ordinary collection contracts; arbitrary __index implementations
    // need not return the same value on successive reads.
    match &owner_type {
        LuaType::Array(_) | LuaType::TableGeneric(_) => {}
        LuaType::Generic(generic)
            if generic.get_base_type_id() == crate::LuaTypeDeclId::global("table")
                && model
                    .type_def_of(&generic.get_base_type_id())
                    .is_some_and(|def| model.is_std_file(def.file_id)) => {}
        _ => return None,
    }
    let mut base = model.type_of_expr(index.get_syntax_id());
    let mut narrowed = false;
    if contains_call(model, index.ancestors::<LuaStat>().next()?.syntax(), offset) {
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
                if LuaClosureId::from_node(condition.syntax()) != closure {
                    return narrowed.then_some(base);
                }
                let primitive = super::primitive_guard_target(
                    model,
                    &condition,
                    matches!(node.kind, FlowNodeKind::TrueCondition(_)),
                )
                .filter(|(target, _, _)| access_matches(model, target, &access));
                if condition.get_range().end() <= offset
                    && let Some((_, primitive, keep_matching)) = primitive
                {
                    base = super::filter_type_by_primitive(model, base, &primitive, keep_matching);
                    narrowed = true;
                } else {
                    if contains_call(model, condition.syntax(), offset) {
                        return narrowed.then_some(base);
                    }
                    if condition.get_range().end() <= offset
                        && excludes_nil(
                            model,
                            &condition,
                            &access,
                            matches!(node.kind, FlowNodeKind::TrueCondition(_)),
                        )
                    {
                        base = super::remove_type(base, &LuaType::Nil);
                        narrowed = true;
                    }
                }
            }
            FlowNodeKind::Assignment(ptr) => {
                let assignment = ptr.to_node(&chunk)?;
                if assignment.get_range().end() <= offset {
                    if LuaClosureId::from_node(assignment.syntax()) != closure {
                        return narrowed.then_some(base);
                    }
                    let (vars, _) = assignment.get_var_and_expr_list();
                    if vars
                        .iter()
                        .any(|var| matches!(var, LuaVarExpr::IndexExpr(_)))
                    {
                        return narrowed.then_some(base);
                    }
                }
            }
            FlowNodeKind::CallExprStat(ptr) => {
                if contains_call(model, ptr.to_node(&chunk)?.syntax(), offset) {
                    return narrowed.then_some(base);
                }
            }
            FlowNodeKind::LoopLabel | FlowNodeKind::NamedLabel(_) | FlowNodeKind::ImplFunc(_) => {
                return narrowed.then_some(base);
            }
            _ => {}
        }
        for effect in tree.get_flow_effects(current) {
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                if value_syntax.get_range().end() <= offset && access.depends_on(decl) {
                    return narrowed.then_some(base);
                }
                let value = value_syntax.to_node_from_root(chunk.syntax())?;
                if contains_call(model, &value, offset) {
                    return narrowed.then_some(base);
                }
            }
        }
        let Some(FlowAntecedent::Single(next)) = node.antecedent else {
            return narrowed.then_some(base);
        };
        current = next;
    }
    narrowed.then_some(base)
}

fn access(model: &SemanticModel, expr: &LuaExpr) -> Option<Access> {
    match expr {
        LuaExpr::NameExpr(name) => {
            let root = model.resolve_name(name.get_position())?;
            model
                .file_facts()?
                .decl_by_id(&root)?
                .kind
                .is_local()
                .then_some(Access {
                    root,
                    parts: Vec::new(),
                })
        }
        LuaExpr::IndexExpr(index) => {
            let mut result = access(model, &index.get_prefix_expr()?)?;
            let part = match index.get_index_key()? {
                LuaIndexKey::Name(name) => {
                    Part::Member(LuaMemberKey::Name(name.get_name_text().into()))
                }
                LuaIndexKey::Expr(LuaExpr::NameExpr(name)) => {
                    let decl = model.resolve_name(name.get_position())?;
                    if !model.file_facts()?.decl_by_id(&decl)?.kind.is_local() {
                        return None;
                    }
                    Part::Local(decl)
                }
                _ => return None,
            };
            result.parts.push(part);
            Some(result)
        }
        LuaExpr::ParenExpr(paren) => access(model, &paren.get_expr()?),
        _ => None,
    }
}

fn access_matches(model: &SemanticModel, target: &LuaExpr, expected: &Access) -> bool {
    access(model, target).as_ref() == Some(expected)
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

fn contains_call(model: &SemanticModel, node: &LuaSyntaxNode, before: TextSize) -> bool {
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        match node.kind().into() {
            LuaSyntaxKind::ClosureExpr => {}
            _ if node.text_range().end() <= before
                && emmylua_parser::LuaCallExpr::cast(node.clone())
                    .is_some_and(|call| !primitive_read_pure_call(model, &call)) =>
            {
                return true;
            }
            _ => pending.extend(node.children()),
        }
    }
    false
}

fn primitive_read_pure_call(model: &SemanticModel, call: &emmylua_parser::LuaCallExpr) -> bool {
    let Some(LuaExpr::NameExpr(callee)) = call.get_prefix_expr() else {
        return false;
    };
    let Some(SemanticId::Decl(decl)) = model.resolve_name(callee.get_position()) else {
        return false;
    };
    if !model.is_std_file(decl.file_id) {
        return false;
    }
    match callee.get_name_text().as_deref() {
        Some("type") => true,
        Some("tostring") => call
            .get_args_list()
            .and_then(|args| args.get_args().next())
            .is_some_and(|arg| {
                matches!(
                    model.type_of_expr_at(arg.get_syntax_id(), arg.get_position()),
                    LuaType::Nil
                        | LuaType::Boolean
                        | LuaType::BooleanConst(_)
                        | LuaType::DocBooleanConst(_)
                        | LuaType::String
                        | LuaType::StringConst(_)
                        | LuaType::DocStringConst(_)
                        | LuaType::Number
                        | LuaType::Integer
                        | LuaType::IntegerConst(_)
                        | LuaType::DocIntegerConst(_)
                        | LuaType::FloatConst(_)
                )
            }),
        _ => false,
    }
}
