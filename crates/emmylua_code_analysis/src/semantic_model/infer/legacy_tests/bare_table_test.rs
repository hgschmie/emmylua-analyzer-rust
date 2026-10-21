use crate::{DiagnosticCode, FileId, LuaType, VirtualWorkspace};
use emmylua_parser::{LuaAstToken, LuaLocalName};

fn result_type(ws: &VirtualWorkspace, file: FileId) -> LuaType {
    let result = ws.get_node::<LuaLocalName>(file);
    let model = ws.analysis.semantic_model(file);
    let decl = model
        .decl_by_offset(result.get_name_token().unwrap().get_position())
        .unwrap();
    model.type_of_decl(&decl).unwrap()
}

#[test]
fn bare_table_members_are_any_for_static_and_dynamic_keys() {
    for expression in [
        "value.missing",
        "value['missing']",
        "value[1]",
        "value[key]",
        "value[number_key]",
        "value[object_key]",
        "value[true]",
        "value.missing.nested",
    ] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
            ---@param value table
            ---@param key string
            ---@param number_key integer
            ---@param object_key table
            function read(value, key, number_key, object_key)
                local result = {expression}
                return result
            end
        "#
        ));
        assert_eq!(result_type(&ws, file), LuaType::Any, "{expression}");
    }
}

#[test]
fn bare_table_conditions_do_not_assume_conditional_writes_exist() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        r#"
        ---@generic T
        ---@param value T
        ---@return T
        local function copy(value) return value end

        ---@param value table
        local function update(value)
            local p = copy(value)
            p.name = 'new'
            if p.minable and p.minable.result then p.minable.result = 'new' end
            if p.place_result then p.place_result = 'new' end
            if p.result then p.result = 'new' end
            return p
        end
        return update
    "#
    ));
}

#[test]
fn bare_table_fix_preserves_known_record_and_array_types() {
    let mut ws = VirtualWorkspace::new();
    ws.def(
        r#"
        ---@class Record
        ---@field name string
        local Record = {}
        ---@type Record
        record = {}
        ---@type string[]
        array = {}
        literal = { count = 1 }
    "#,
    );
    assert_eq!(ws.expr_ty("record.name"), LuaType::String);
    assert_eq!(ws.expr_ty("array[1]"), ws.ty("string?"));
    assert_eq!(ws.expr_ty("literal.missing"), LuaType::Nil);
}

#[test]
fn bare_table_alias_members_are_any() {
    for expression in ["value.missing", "value[key]"] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!(
            r#"
            ---@alias Bag table
            ---@alias OtherBag Bag
            ---@param value OtherBag
            ---@param key string
            function lookup(value, key)
                local result = {expression}
                return result
            end
        "#
        ));
        assert_eq!(result_type(&ws, file), LuaType::Any, "{expression}");
    }
}

#[test]
fn bare_table_generic_member_call_preserves_open_table() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(
        r#"
        ---@generic T
        ---@param object T
        ---@return T
        function table.deepcopy(object) end
    "#,
    );
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        r#"
        ---@param prototype table
        ---@param new_name string
        local function update(prototype, new_name)
            if not prototype.type or not prototype.name then error('Invalid prototype') end
            local p = table.deepcopy(prototype)
            p.name = new_name
            if p.minable and p.minable.result then p.minable.result = new_name end
            if p.place_result then p.place_result = new_name end
            if p.result then p.result = new_name end
            return p
        end
        return update
    "#
    ));
}

#[test]
fn bare_table_member_guards_preserve_both_branches() {
    for (annotation, expected) in [("table", LuaType::Table), ("any", LuaType::Any)] {
        for condition in [
            "value.kind",
            "not value.kind",
            "value.kind == 'a'",
            "value.kind ~= 'a'",
            "value.kind == nil",
            "value.kind ~= nil",
        ] {
            let mut ws = VirtualWorkspace::new();
            let file = ws.def(&format!(
                r#"
                ---@param value {annotation}
                function guarded(value)
                    if {condition} then
                        local result = value
                        return result
                    end
                end
            "#
            ));
            assert_eq!(
                result_type(&ws, file),
                expected,
                "{annotation}: {condition}"
            );
        }
    }
}

#[test]
fn bare_table_union_and_long_alias_reads_remain_open() {
    for annotation in ["table|Record", "OpenUnion", "Chain47"] {
        let mut definitions = String::from(
            "---@class Record\n---@field known string\n---@alias OpenUnion table|Record\n---@alias Chain0 table\n",
        );
        for n in 1..48 {
            definitions.push_str(&format!("---@alias Chain{n} Chain{}\n", n - 1));
        }
        for expression in ["value.missing", "value[key]"] {
            let mut ws = VirtualWorkspace::new();
            ws.def(&definitions);
            let file = ws.def(&format!("---@param value {annotation}\n---@param key string\nfunction read(value,key) local result={expression} return result end"));
            assert_eq!(
                result_type(&ws, file),
                LuaType::Any,
                "{annotation}: {expression}"
            );
        }
    }
}

#[test]
fn bare_table_generic_repair_keeps_nested_arguments_and_intersections() {
    let mut ws = VirtualWorkspace::new();
    ws.def("---@class Wrapper<A,B>\n---@field first A\n---@field second B\n---@class Owner<T>\n---@field value Wrapper<{foo:T},T>\n---@field nested Wrapper<Wrapper<{foo:T},T>,T>\n---@class Record\n---@field known string");
    for (annotation, expression, expected) in [
        ("Owner<integer>", "value.value.first.foo", LuaType::Integer),
        ("Owner<integer>", "value.value.second", LuaType::Integer),
        (
            "Owner<integer>",
            "value.nested.first.first.foo",
            LuaType::Integer,
        ),
        ("Owner<integer>", "value.nested.second", LuaType::Integer),
    ] {
        let file=ws.def(&format!("---@param value {annotation}\nfunction read(value) local result={expression} return result end"));
        assert_eq!(
            result_type(&ws, file),
            expected,
            "{annotation}: {expression}"
        );
    }
}

#[test]
fn bare_table_intersection_keeps_its_structured_member_contract() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def("---@class Record\n---@field known string");
    let ty = ws.ty("table&Record");
    let model = ws.analysis.semantic_model(file);
    assert!(!crate::semantic_model::member::has_unrestricted_members(
        &model, &ty
    ));
    assert_eq!(
        model.member_type(&ty, &crate::LuaMemberKey::from("known")),
        Some(LuaType::String)
    );
}

#[test]
fn bare_table_generic_repair_preserves_union_object_arguments() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def(
        r#"
---@class Wrapper<A,B>
---@field first A
---@field second B
---@class Owner<T>
---@field value Wrapper<{foo:T}|string,T>
"#,
    );
    let owner = ws.ty("Owner<integer>");
    let model = ws.analysis.semantic_model(file);
    let value = model
        .member_type(&owner, &crate::LuaMemberKey::from("value"))
        .unwrap();
    let first = model
        .member_type(&value, &crate::LuaMemberKey::from("first"))
        .unwrap();
    let LuaType::Union(union) = first else {
        panic!("expected union: {first:?}");
    };
    let branches = union.into_vec();
    assert!(branches.contains(&LuaType::String));
    let object = branches
        .iter()
        .find_map(|ty| match ty {
            LuaType::Object(object) => Some(object),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        object.get_field(&crate::LuaMemberKey::from("foo")),
        Some(&LuaType::Integer)
    );
    assert_eq!(
        model.member_type(&value, &crate::LuaMemberKey::from("second")),
        Some(LuaType::Integer)
    );
}
