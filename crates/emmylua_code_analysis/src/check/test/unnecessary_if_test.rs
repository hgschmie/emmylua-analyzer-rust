#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_vararg_unknown_and_boolean_conditions_are_not_constant() {
        let diagnostics = crate::check::test::check_source(
            r#"
            local function unknown(...)
                if ... then print('present') end
                assert(...)
            end
            ---@param ... boolean
            local function boolean(...)
                if ... then print('true') end
                assert(...)
            end
            unknown(false)
            boolean(false)
        "#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn test_vararg_false_conditions_still_report_impossible_branches() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param ... false
            local function always_false(...)
                if ... then print('unreachable') end
                assert(...)
            end
            always_false(false)
        "#,
        );
        let messages: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.code, d.message.as_str()))
            .collect();
        assert_eq!(
            messages,
            vec![
                (
                    DiagnosticCode::UnnecessaryAssert,
                    "Impossible assert: this expression is always falsy; prefer `error()`"
                ),
                (
                    DiagnosticCode::UnnecessaryIf,
                    "Impossible `if` statement: this condition is always falsy"
                ),
            ]
        );
    }

    #[test]
    fn test_issue_392() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf,
        r#"
        local a = false ---@type boolean|nil
        if a == nil or a then -- Unnecessary `if` statement: this condition is always truthy [unnecessary-if]
            print('a is not false')
        end
        "#
        ));
    }

    #[test]
    fn test_issue_396() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf,
        r#"
        local a = false ---@type 'a'|'b'
        if a ~= 'a' then -- Unnecessary `if` statement: this condition is always truthy [unnecessary-if]
        end
        "#
        ));
    }
}

#[test]
fn test_inferred_computed_table_writes() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (body, expected) in [
        (
            "local seen = {}; seen[key] = true; local observed = seen[key]",
            LuaType::BooleanConst(true),
        ),
        (
            "local seen = {}; if flag then seen[key] = true end; local observed = seen[key]",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
        (
            "local seen = {}; seen[other] = true; local observed = seen[key]",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
        (
            "local seen = {}; for i = 1, 3 do local observed = seen[key]; seen[key] = true end",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
        ("local seen = {}; local observed = seen[key]", LuaType::Nil),
        (
            "local seen = {}; seen[key] = true; seen[key] = nil; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "for i = 1, 3 do local seen = {}; local observed = seen[key]; seen[key] = true end",
            LuaType::Nil,
        ),
        (
            "local seen = {}; local different = {}; different[key] = true; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "local seen = {}; seen[key] = true; seen = {}; local observed = seen[key]",
            LuaType::Nil,
        ),
    ] {
        let source = format!(
            r#"
        ---@param key string
        ---@param other string
        ---@param flag boolean
        local function probe(key, other, flag) {body} end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let pos = source.find("local observed = ").unwrap() + "local observed = ".len();
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|i| u32::from(i.get_position()) as usize == pos)
            .unwrap();
        let ty = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        assert_eq!(ty, expected, "{body}");
    }
}

#[test]
fn test_inferred_computed_table_writes_loop_condition() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        r#"
        ---@param keys string[]
        local function create(keys)
            local seen = {}
            for _, key in ipairs(keys) do
                ---@cast key string
                if seen[key] then print('duplicate') else seen[key] = true end
            end
        end
    "#
    ));
}

#[test]
fn test_inferred_computed_table_writes_key_and_result_slots() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (body, expected) in [
        (
            "local key = 'wanted'; local seen = {}; seen.other = true; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "local key = 'wanted'; local other = 'different'; local seen = {}; seen[other] = true; local observed = seen[key]",
            LuaType::Nil,
        ),
        (
            "local seen = {}; seen[key] = pair(); local observed = seen[key]",
            LuaType::String,
        ),
        (
            "local seen = {}; local spare; seen[key], spare = pair(); local observed = seen[key]",
            LuaType::String,
        ),
        (
            "local seen = {}; local spare; spare, seen[key] = pair(); local observed = seen[key]",
            LuaType::Integer,
        ),
        (
            "local seen = {}; seen[key] = true; local alias = seen; seen = {}; local observed = alias[key]",
            LuaType::BooleanConst(true),
        ),
    ] {
        let source = format!(
            r#"
        ---@return string, integer
        local function pair() return 'value', 7 end
        ---@param key string
        local function probe(key) {body} end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let pos = source.find("local observed = ").unwrap() + "local observed = ".len();
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .find(|i| u32::from(i.get_position()) as usize == pos)
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            expected,
            "{body}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes_finite_key_domains() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for key_type in ["'a'|'b'", "FiniteComputedKeys"] {
        let source = format!(
            r#"
        ---@alias FiniteComputedKeys 'a'|'b'
        ---@param key {key_type}
        local function probe(key)
            local seen = {{}}
            seen.other = true
            local observed = seen[key]
        end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            LuaType::Nil,
            "{key_type}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes_recursive_key_domains() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (alias, key_type) in [
        ("---@alias RecursiveKey 'a'|RecursiveKey", "RecursiveKey"),
        ("---@alias RecursiveKey RecursiveKey|'a'", "RecursiveKey"),
        (
            "---@alias GrowingKey<T> T|GrowingKey<T[]>",
            "GrowingKey<'a'>",
        ),
    ] {
        let source = format!(
            r#"
        {alias}
        ---@param key {key_type}
        local function probe(key)
            local seen = {{}}
            seen.other = true
            local observed = seen[key]
        end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        let ty = model.type_of_expr_at(index.get_syntax_id(), index.get_position());
        assert!(
            ty.is_unknown()
                || ty == LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
            "unresolved key domain must retain possible writes: {alias}: {ty:?}"
        );
    }
}

#[test]
fn test_inferred_computed_table_writes_written_key_domains() {
    use crate::{LuaType, VirtualWorkspace};
    use emmylua_parser::{LuaAstNode, LuaIndexExpr};
    for (other_type, expected) in [
        ("'a'|'b'", LuaType::Nil),
        ("FiniteWrittenKeys", LuaType::Nil),
        ("integer", LuaType::Nil),
        (
            "RecursiveWrittenKey",
            LuaType::from_vec(vec![LuaType::BooleanConst(true), LuaType::Nil]),
        ),
    ] {
        let source = format!(
            r#"
        ---@alias FiniteWrittenKeys 'a'|'b'
        ---@alias RecursiveWrittenKey 'a'|RecursiveWrittenKey
        ---@param key 'wanted'
        ---@param other {other_type}
        local function probe(key, other)
            local seen = {{}}
            seen[other] = true
            local observed = seen[key]
        end
        "#
        );
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&source);
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(index.get_syntax_id(), index.get_position()),
            expected,
            "{other_type}"
        );
    }
}
