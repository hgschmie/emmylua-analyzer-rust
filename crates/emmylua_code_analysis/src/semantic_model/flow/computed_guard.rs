//! Condition facts for stable computed accesses, which have no static member ID.

use emmylua_parser::{
    BinaryOperator, LuaAstNode, LuaExpr, LuaIndexExpr, LuaIndexKey, LuaLiteralToken, LuaStat,
    UnaryOperator,
};
use rowan::TextSize;

use super::{SemanticModel, computed_effects};
use crate::semantic_db::def::SemanticId;
use crate::semantic_db::flow::{FlowAntecedent, FlowEffect, FlowNodeKind, LuaClosureId};
use crate::{LuaMemberKey, LuaType};

#[derive(PartialEq, Eq)]
pub(super) enum Part {
    Member(LuaMemberKey),
    Key(Box<Access>),
}

#[derive(PartialEq, Eq)]
pub(super) struct Access {
    pub(super) root: SemanticId,
    pub(super) parts: Vec<Part>,
}

impl Access {
    pub(super) fn depends_on(&self, decl: &SemanticId) -> bool {
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
    // A computed key can select declared class fields as well as collection entries.
    // Explicit __index handlers can change values between reads and remain excluded.
    match &owner_type {
        LuaType::Array(_) | LuaType::TableGeneric(_) => {}
        LuaType::Generic(generic)
            if generic.get_base_type_id() == crate::LuaTypeDeclId::global("table")
                && model
                    .type_def_of(&generic.get_base_type_id())
                    .is_some_and(|def| model.is_std_file(def.file_id)) => {}
        _ if (!crate::semantic_model::inherited_index::dictionaries(model, &owner_type)
            .is_empty()
            || model
                .member_infos(&owner_type)
                .iter()
                .any(|member| member.id.is_some()))
            && !model
                .member_infos(&owner_type)
                .iter()
                .any(|member| member.key == LuaMemberKey::Name("__index".into())) => {}
        _ => return None,
    }
    let inferred = model.type_of_expr(index.get_syntax_id());
    let mut base = match (&inferred, &owner_type) {
        // Recursive element inference can be unknown at a loop/write site. The
        // owner's array contract is still available, as in guarded_index_type.
        (LuaType::Unknown, LuaType::Array(array)) => {
            LuaType::from_vec(vec![array.get_base().clone(), LuaType::Nil])
        }
        _ => inferred,
    };
    // Ordinary projection omits nil for some optional named reference fields.
    // Apply their full contract before deciding whether a guard is needed.
    if let Some(key) = super::member_key_from_index_expr(model, index)
        && let Some(member) = model.member_info(&owner_type, &key)
        && let Some(id) = member.id
    {
        base = super::apply_field_nullable(model, &id, base);
    }

    if !base.is_nullable() {
        return None;
    }
    if computed_effects::calls_conflict(
        model,
        &access,
        index.ancestors::<LuaStat>().next()?.syntax(),
        offset,
    ) {
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
                    || computed_effects::calls_conflict(model, &access, condition.syntax(), offset)
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
                    if computed_effects::calls_conflict(model, &access, assignment.syntax(), offset)
                    {
                        return None;
                    }
                    let (vars, _) = assignment.get_var_and_expr_list();
                    if vars
                        .iter()
                        .any(|var| computed_effects::write_conflicts(model, &access, var, offset))
                    {
                        return None;
                    }
                }
            }
            FlowNodeKind::CallExprStat(ptr) => {
                if computed_effects::calls_conflict(
                    model,
                    &access,
                    ptr.to_node(&chunk)?.syntax(),
                    offset,
                ) {
                    return None;
                }
            }
            FlowNodeKind::NamedLabel(_) | FlowNodeKind::ImplFunc(_) => {
                return None;
            }
            _ => {}
        }
        for effect in tree.get_flow_effects(current) {
            if let FlowEffect::LoopEntry(syntax) = effect {
                let loop_node = syntax.to_node_from_root(chunk.syntax())?;
                if computed_effects::loop_conflicts(model, &access, &loop_node) {
                    return None;
                }
            }
            if let FlowEffect::AssignDecl { decl, value_syntax } = effect {
                if value_syntax.get_range().end() <= offset && access.depends_on(decl) {
                    return None;
                }
                let value = value_syntax.to_node_from_root(chunk.syntax())?;
                if computed_effects::calls_conflict(model, &access, &value, offset) {
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

// Both guard and assignment facts require ordinary collection semantics.
pub(super) fn ordinary_collection(model: &SemanticModel, owner_type: &LuaType) -> bool {
    match &owner_type {
        LuaType::Array(_) | LuaType::TableGeneric(_) => true,
        LuaType::Generic(generic)
            if generic.get_base_type_id() == crate::LuaTypeDeclId::global("table")
                && model
                    .type_def_of(&generic.get_base_type_id())
                    .is_some_and(|def| model.is_std_file(def.file_id)) =>
        {
            true
        }
        _ if !crate::semantic_model::inherited_index::dictionaries(model, &owner_type)
            .is_empty()
            && !model
                .member_infos(&owner_type)
                .iter()
                .any(|member| member.key == LuaMemberKey::Name("__index".into())) =>
        {
            true
        }
        _ => false,
    }
}

// Globals use their binding name rather than a particular declaration site.
// Locals retain declaration identity, keeping shadowed names separate.
pub(super) fn reference(
    model: &SemanticModel,
    name: &emmylua_parser::LuaNameExpr,
) -> Option<SemanticId> {
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

pub(super) fn access(model: &SemanticModel, expr: &LuaExpr) -> Option<Access> {
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

#[cfg(test)]
mod class_field_tests {
    use crate::{DiagnosticCode, VirtualWorkspace};
    fn check(body: &str, safe: bool) {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let src = format!(
            r#"
---@class Record
---@field value integer
---@class Registry
---@field first Record?
---@field second Record?
---@alias Key 'first'|'second'
---@param registry Registry
---@param key Key
---@param other Key
local function f(registry, key, other)
{body}
end
"#
        );
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &src),
            safe,
            "{body}"
        );
    }
    #[test]
    fn computed_class_fields_keep_matching_guards() {
        for body in [
            "if registry[key] then mutate(); print(registry[key].value) end",
            "if registry[key] then print(registry[key].value) end",
            "local value = registry[key] and registry[key].value",
            "if not (registry[key] and registry[key].value) then return end",
        ] {
            check(body, true);
        }
    }
    #[test]
    fn computed_class_fields_reject_stale_or_different_guards() {
        for body in [
            "print(registry[key].value)",
            "if registry[key] then key = other; print(registry[key].value) end",
            "if registry[key] then print(registry[other].value) end",
            "if registry[key] then registry[key] = nil; print(registry[key].value) end",
        ] {
            check(body, false);
        }
    }
}

#[cfg(test)]
mod effect_boundary_tests {
    use crate::{DiagnosticCode, VirtualWorkspace};

    fn check(body: &str, safe: bool) {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let src = format!(
            r#"
---@param rows integer[][]
---@param key integer
---@param other integer
---@param predicate fun():boolean
local function f(rows, key, other, predicate)
{body}
end
"#
        );
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, &src),
            safe,
            "{body}"
        );
    }

    #[test]
    fn computed_facts_survive_opaque_calls_and_child_writes() {
        for body in [
            "if rows[key] then predicate(); print(rows[key][1]) end",
            "rows[key] = rows[key] or {}; local v = rows[key][other] or predicate() or 0; rows[key][other] = v",
            "if rows[key] then rows[key][other] = nil; print(rows[key][1]) end",
            "if rows[key] then local out = {}; out[other] = 1; print(rows[key][1]) end",
            "if rows[key] then local deferred = function() rows[key] = nil end; print(rows[key][1]) end",
        ] {
            check(body, true);
        }
    }

    #[test]
    fn global_dictionary_initialization_survives_callback() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
---@class Storage
---@field capacity table<string, table<string, number>?>
---@type Storage
storage = {}
---@param name string
---@param quality string
---@param callback fun():number?
local function capacity(name, quality, callback)
    storage.capacity[name] = storage.capacity[name] or {}
    local value = storage.capacity[name][quality] or callback() or 0
    storage.capacity[name][quality] = value
    return math.floor(value)
end
"#
        ));
    }

    #[test]
    fn computed_facts_cross_nonconflicting_nested_loop() {
        check(
            r#"
local result = {}
for i = 1, 32 do
    if rows[i] and predicate() then
        for id in pairs(rows[i]) do
            if predicate() then result[id] = 1
            else rows[i][id] = nil end
        end
    end
end
return result
"#,
            true,
        );
        check(
            "rows[key] = {}; for id = 1, 10 do rows[key][id] = id end",
            true,
        );
    }

    #[test]
    fn computed_facts_stop_at_visible_conflicts() {
        for body in [
            "if rows[key] then rawset(rows, key, nil); print(rows[key][1]) end",
            "if rows[key] then rows[key][other] = rawset(rows, key, nil); print(rows[key][1]) end",
            "rows[key] = {}; rawset(rows, key, nil); print(rows[key][1])",
            "rows[key] = {}; for i = 1, 10 do print(rows[key][1]); rows[key] = nil end",
            "local function clear() rows[key] = nil end; rows[key] = {}; clear(); print(rows[key][1])",
            "if rows[key] then setmetatable(rows, {}); print(rows[key][1]) end",
            "if rows[key] then local other = {}; other = rows; other[key] = nil; print(rows[key][1]) end",
            "if rows[key] then table.remove(rows, key); print(rows[key][1]) end",
            "local function clear() rows[key] = nil end; if rows[key] then clear(); print(rows[key][1]) end",
            "local clear = function() rows[key] = nil end; if rows[key] then clear(); print(rows[key][1]) end",
            "if rows[key] then (function() rows[key] = nil end)(); print(rows[key][1]) end",
            "if rows[key] then for i = 1, 10 do print(rows[key][1]); rows[key] = nil end end",
            "if rows[key] then for i = 1, 10 do print(rows[key][1]); key = other end end",
            "local alias = rows; if rows[key] then for i = 1, 10 do print(rows[key][1]); alias[key] = nil end end",
            "local out = {}; escape(out); if rows[key] then out[other] = nil; print(rows[key][1]) end",
            "if rows[key] then for i = 1, 10 do print(rows[key][1]); rawset(rows, key, nil) end end",
        ] {
            check(body, false);
        }
    }
}
