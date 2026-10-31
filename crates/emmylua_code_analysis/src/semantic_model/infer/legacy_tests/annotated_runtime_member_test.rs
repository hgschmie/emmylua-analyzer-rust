use crate::{DiagnosticCode, LuaType, VirtualWorkspace};
use googletest::prelude::*;

#[gtest]
fn annotated_runtime_member_dictionary_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type table<string,string>\nC.names = {}");
    let expected = ws.ty("table<string,string>");
    expect_that!(ws.expr_ty("C.names"), eq(&expected));
}

#[gtest]
fn annotated_runtime_member_array_chain() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type string[]\nC.names = {}");
    let expected = ws.ty("string?");
    expect_that!(ws.expr_ty("C.names[1]"), eq(&expected));
}

#[gtest]
fn annotated_runtime_member_cross_file_class() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file("config.lua", "---@class Config\n---@field name string\nlocal C = {}\n---@type Config\nC.config = {}\nreturn C");
    expect_that!(
        ws.expr_ty("require('config').config.name"),
        eq(&LuaType::String)
    );
}

#[gtest]
fn annotated_runtime_member_does_not_hide_invalid_initializer() {
    let mut ws = VirtualWorkspace::new();
    expect_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            "local C = {}\n---@type string\nC.name = 42"
        ),
        eq(false)
    );
}

#[gtest]
fn annotated_runtime_member_nullable_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type string?\nC.name = nil");
    let expected = ws.ty("string?");
    expect_that!(ws.expr_ty("C.name"), eq(&expected));
}

#[gtest]
fn annotated_runtime_member_object_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type {name: string}\nC.config = {}");
    expect_that!(ws.expr_ty("C.config.name"), eq(&LuaType::String));
}

#[gtest]
fn annotated_runtime_member_function_contract() {
    let mut ws = VirtualWorkspace::new();
    ws.def("C = {}\n---@type fun():string\nC.name = function() return 'one' end");
    expect_that!(ws.expr_ty("C.name()"), eq(&LuaType::String));
}

#[gtest]
fn annotated_runtime_member_named_string_field_is_not_an_index() {
    let mut ws = VirtualWorkspace::new();
    ws.def("---@class Config\n---@field string string\nC = {}\n---@type Config\nC.config = {}");
    expect_that!(
        ws.has_no_diagnostic(DiagnosticCode::InjectField, "C.config.extra = true"),
        eq(false)
    );
}

#[gtest]
fn annotated_runtime_member_index_contracts_validate_keys_and_named_class_values() {
    for contract in ["NumericRecord", "{[integer]: string, named: boolean}"] {
        let mut cases = vec![
            ("C.config[1] = 'ok'", vec![]),
            ("C.config.named = false", vec![]),
            ("C.config['1'] = 'ok'", vec![DiagnosticCode::InjectField]),
            ("C.config.extra = true", vec![DiagnosticCode::InjectField]),
        ];
        if contract == "NumericRecord" {
            cases.push((
                "C.config.named = 'wrong'",
                vec![DiagnosticCode::AssignTypeMismatch],
            ));
        }
        for (operation, expected) in cases {
            let source = format!(
                r#"
                ---@class NumericRecord
                ---@field [integer] string
                ---@field named boolean
                local Record = {{}}
                local C = {{}}
                ---@type {contract}
                C.config = {{ named = true }}
                {operation}
                return C
            "#
            );
            for code in [
                DiagnosticCode::AssignTypeMismatch,
                DiagnosticCode::InjectField,
            ] {
                let mut ws = VirtualWorkspace::new();
                expect_that!(
                    ws.has_no_diagnostic(code, &source),
                    eq(!expected.contains(&code)),
                    "{contract}, {operation}: {code:?}"
                );
            }
        }
    }
}

#[gtest]
fn annotated_runtime_member_shell_and_rich_queries_agree() {
    use crate::LuaMemberKey;
    let mut ws = VirtualWorkspace::new();
    let file = ws.def("local C = {}\n---@type table<string,string>\nC.names = {}\nreturn C");
    let foreign_file = ws.def("local unrelated = true");
    let expected = ws.ty("table<string,string>");
    for model_file in [file, foreign_file] {
        for rich_first in [false, true] {
            let model = ws.analysis.semantic_model(model_file);
            let facts = model.file_facts_of(file).unwrap();
            let member = facts
                .members
                .iter()
                .find(|member| member.key == LuaMemberKey::from("names"))
                .unwrap();
            assert!(member.doc_type_syntax.is_some() && member.value_syntax.is_some());
            assert_ne!(member.doc_type_syntax, member.value_syntax);
            if rich_first {
                expect_that!(
                    model.type_of_member(&member.id),
                    eq(&Some(expected.clone()))
                );
            }
            let shell = model
                .analysis()
                .member_type(file, member.id.clone())
                .unwrap();
            let projected = model.analysis().type_shell_lua(file, &shell);
            expect_that!(
                projected,
                eq(&expected),
                "foreign={}, rich_first={rich_first}",
                model_file != file
            );
            expect_that!(
                model.type_of_member(&member.id),
                eq(&Some(expected.clone()))
            );
        }
    }
}

#[gtest]
fn annotated_runtime_member_inline_annotation_preserves_initializer_checks() {
    for (value, valid) in [("'ready'", true), ("42", false)] {
        let source = format!("C = {{\n---@type string\nname = {value}\n}}");
        let mut ws = VirtualWorkspace::new();
        ws.def(&source);
        expect_that!(ws.expr_ty("C.name"), eq(&LuaType::String), "{value}");
        let mut ws = VirtualWorkspace::new();
        expect_that!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
            eq(valid),
            "{value}"
        );
    }
}

#[gtest]
fn annotated_runtime_member_projection_preserves_supported_shapes() {
    for (annotation, initializer, shell_annotation) in [
        ("string?", "nil", "string?"),
        (
            "fun(value:string):integer",
            "function(value) return 1 end",
            "fun(value:string):integer",
        ),
        ("table<string,string>", "{}", "table<string,string>"),
        ("{name:string}", "{}", "table"),
        ("{[integer]:string,named:boolean}", "{named=true}", "table"),
    ] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!(
            "local C={{}}\n---@type {annotation}\nC.item={initializer}\nreturn C"
        ));
        let foreign = ws.def("local unrelated=true");
        let expected_rich = ws.ty(annotation);
        let expected_shell = ws.ty(shell_annotation);
        for model_file in [file, foreign] {
            for rich_first in [false, true] {
                let model = ws.analysis.semantic_model(model_file);
                let member = model
                    .file_facts_of(file)
                    .unwrap()
                    .members
                    .iter()
                    .find(|m| m.key == crate::LuaMemberKey::from("item"))
                    .unwrap();
                if rich_first {
                    expect_that!(
                        model.type_of_member(&member.id),
                        eq(&Some(expected_rich.clone()))
                    );
                }
                let shell = model
                    .analysis()
                    .member_type(file, member.id.clone())
                    .unwrap();
                expect_that!(
                    model.analysis().type_shell_lua(file, &shell),
                    eq(&expected_shell),
                    "{annotation}"
                );
                expect_that!(
                    model.type_of_member(&member.id),
                    eq(&Some(expected_rich.clone())),
                    "{annotation}"
                );
            }
        }
    }
}

#[gtest]
fn annotated_runtime_member_class_generics_keep_owner_indices() {
    for annotation in ["U", "{value:U}", "fun():U"] {
        for placement in ["assignment", "inline", "wrapped", "trailing"] {
            let body = match placement {
                "assignment" => format!("local C={{}}\n---@type {annotation}\nC.item=nil"),
                "inline" => format!("local C={{\n---@type {annotation}\nitem=nil\n}}"),
                "wrapped" => format!("local C=(({{\n---@type {annotation}\nitem=nil\n}}))"),
                _ => format!(
                    "---@type Box, Other\nlocal C,trailing=(({{\n---@type {annotation}\nitem=nil\n}}))"
                ),
            };
            let mut ws = VirtualWorkspace::new();
            let file=ws.def(&format!("---@class Other\nlocal Other={{}}\n---@class Box<T:integer,U:string>\n{body}\nreturn C"));
            let foreign = ws.def("local unrelated=true");
            for model_file in [file, foreign] {
                let model = ws.analysis.semantic_model(model_file);
                let member = model
                    .file_facts_of(file)
                    .unwrap()
                    .members
                    .iter()
                    .find(|m| m.key == crate::LuaMemberKey::from("item"))
                    .unwrap();
                let rich = model.type_of_member(&member.id).unwrap();
                let value = match &rich {
                    LuaType::Object(object) => object
                        .get_field(&crate::LuaMemberKey::from("value"))
                        .unwrap(),
                    LuaType::DocFunction(fun) => fun.get_ret(),
                    ty => ty,
                };
                expect_true!(
                    matches!(value,LuaType::TplRef(t) if t.get_name()=="U" && t.get_tpl_id()==crate::GenericTplId::Type(1) && t.get_constraint()==Some(&LuaType::String)),
                    "{placement}, {annotation}, foreign={}: {rich:?}",
                    model_file != file
                );
                if annotation == "U" {
                    let shell = model
                        .analysis()
                        .member_type(file, member.id.clone())
                        .unwrap();
                    expect_true!(shell.candidates.iter().any(|c|matches!(c,crate::semantic_db::types::TypeCandidate::Generic(name) if name=="U")),"{placement}: {shell:?}");
                }
            }
        }
    }
}

#[gtest]
fn annotated_runtime_member_generic_shadowing_stays_scope_local() {
    use crate::semantic_db::types::TypeCandidate;
    for (prefix, suffix, lexical) in [
        (
            "---@generic T\n---@param value T\nlocal function outer(value)\n",
            "end",
            true,
        ),
        (
            "---@generic T\n---@param value T\nlocal function outer(value)\nlocal function inner()\n",
            "end\nend",
            true,
        ),
        (
            "---@generic T\n---@param value T\nlocal function outer(value)\n---@generic U\nlocal function inner()\n",
            "end\nend",
            true,
        ),
        ("---@generic T\nlocal function unrelated() end\n", "", false),
    ] {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!(
            "{prefix}local C={{}}\n---@type T\nC.item=nil\n{suffix}"
        ));
        let model = ws.analysis.semantic_model(file);
        let member = model
            .file_facts()
            .unwrap()
            .members
            .iter()
            .find(|m| m.key == crate::LuaMemberKey::from("item"))
            .unwrap();
        let shell = model
            .analysis()
            .member_type(file, member.id.clone())
            .unwrap();
        expect_that!(
            shell
                .candidates
                .iter()
                .any(|c| matches!(c,TypeCandidate::Generic(name) if name=="T")),
            eq(lexical),
            "{prefix}: {shell:?}"
        );
    }
    for shadowed in [false, true] {
        let mut ws = VirtualWorkspace::new();
        let assignment = "---@type {first:T,second:U}\nC.item={}\n";
        let body = if shadowed {
            format!("---@generic U:boolean\nlocal function set_item()\n{assignment}end")
        } else {
            format!("---@generic U:boolean\nlocal function unrelated() end\n{assignment}")
        };
        let file = ws.def(&format!(
            "---@class Box<T:integer,U:string>\nlocal C={{}}\n{body}"
        ));
        let foreign = ws.def("local unrelated=true");
        for model_file in [file, foreign] {
            let model = ws.analysis.semantic_model(model_file);
            let member = model
                .file_facts_of(file)
                .unwrap()
                .members
                .iter()
                .find(|m| m.key == crate::LuaMemberKey::from("item"))
                .unwrap();
            let rich = model.type_of_member(&member.id).unwrap();
            let LuaType::Object(object) = rich else {
                panic!("expected object contract")
            };
            let first = object
                .get_field(&crate::LuaMemberKey::from("first"))
                .unwrap();
            expect_true!(
                matches!(first,LuaType::TplRef(t) if t.get_name()=="T" && t.get_tpl_id()==crate::GenericTplId::Type(0) && t.get_constraint()==Some(&LuaType::Integer))
            );
            let second = object
                .get_field(&crate::LuaMemberKey::from("second"))
                .unwrap();
            if shadowed {
                expect_true!(
                    matches!(second,LuaType::Ref(id) if id.get_name()=="U"),
                    "mixed lexical binder remains unresolved: {second:?}"
                );
            } else {
                expect_true!(
                    matches!(second,LuaType::TplRef(t) if t.get_name()=="U" && t.get_tpl_id()==crate::GenericTplId::Type(1) && t.get_constraint()==Some(&LuaType::String))
                );
            }
        }
    }
}

#[gtest]
fn annotated_runtime_member_numeric_enum_keeps_literal_priority() {
    for annotation in ["", "integer", "number", "string"] {
        let tag = if annotation.is_empty() {
            String::new()
        } else {
            format!("---@type {annotation}\n")
        };
        for placement in ["plain", "wrapped", "trailing"] {
            let value = format!("{{\n{tag}item=1\n}}");
            let assignment = match placement {
                "plain" => format!("local C={value}"),
                "wrapped" => format!("local C=(({value}))"),
                _ => format!("local C,trailing=(({value}))"),
            };
            let source = format!("---@enum Choice\n{assignment}\nreturn C");
            let mut ws = VirtualWorkspace::new();
            let file = ws.def(&source);
            let foreign = ws.def("local unrelated=true");
            for model_file in [file, foreign] {
                for rich_first in [false, true] {
                    let model = ws.analysis.semantic_model(model_file);
                    let member = model
                        .file_facts_of(file)
                        .unwrap()
                        .members
                        .iter()
                        .find(|m| m.key == crate::LuaMemberKey::from("item"))
                        .unwrap();
                    if rich_first {
                        expect_that!(
                            model.type_of_member(&member.id),
                            eq(&Some(LuaType::IntegerConst(1)))
                        );
                    }
                    let shell = model
                        .analysis()
                        .member_type(file, member.id.clone())
                        .unwrap();
                    expect_that!(
                        model.analysis().type_shell_lua(file, &shell),
                        eq(&LuaType::Integer),
                        "{annotation}"
                    );
                    expect_that!(
                        model.type_of_member(&member.id),
                        eq(&Some(LuaType::IntegerConst(1))),
                        "{annotation}"
                    );
                }
            }
            let mut check_ws = VirtualWorkspace::new();
            expect_that!(
                check_ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
                eq(annotation != "string")
            );
        }
    }
}

#[gtest]
fn annotated_runtime_member_string_template_preserves_scope_indices() {
    for shadowed in [false, true] {
        let assignment = "---@type `U`\nC.item=''\n";
        let body = if shadowed {
            format!("---@generic T:boolean\nlocal function set_item()\n{assignment}end")
        } else {
            assignment.to_string()
        };
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!(
            "---@class Box<T:integer,U:string>\nlocal C={{}}\n{body}"
        ));
        let model = ws.analysis.semantic_model(file);
        let member = model
            .file_facts()
            .unwrap()
            .members
            .iter()
            .find(|m| m.key == crate::LuaMemberKey::from("item"))
            .unwrap();
        let shell = model
            .analysis()
            .member_type(file, member.id.clone())
            .unwrap();
        expect_true!(shell.candidates.iter().any(|c|matches!(c,crate::semantic_db::types::TypeCandidate::StrTpl(t) if t.name=="U" && t.tpl_index==Some(1))),"{shadowed}: {shell:?}");
    }
    for (annotation, name, index) in [
        ("`U`", "U", Some(0)),
        ("fun():`U`", "U", Some(0)),
        ("fun():`T`", "T", None),
    ] {
        use crate::semantic_db::types::TypeCandidate;
        let mut ws = VirtualWorkspace::new();
        let file=ws.def(&format!("---@generic T\nlocal function outer()\n---@generic U\nlocal function inner()\nlocal C={{}}\n---@type {annotation}\nC.item=nil\nend\nend"));
        let model = ws.analysis.semantic_model(file);
        let member = model
            .file_facts()
            .unwrap()
            .members
            .iter()
            .find(|m| m.key == crate::LuaMemberKey::from("item"))
            .unwrap();
        let shell = model
            .analysis()
            .member_type(file, member.id.clone())
            .unwrap();
        let projected = match shell.candidates.first().unwrap() {
            TypeCandidate::Function(fun) => &fun.returns,
            _ => &shell,
        };
        expect_true!(
            projected.candidates.iter().any(
                |c| matches!(c,TypeCandidate::StrTpl(t) if t.name==name && t.tpl_index==index)
            ),
            "{annotation}: {shell:?}"
        );
    }
}

#[gtest]
fn annotated_runtime_member_keeps_outer_generic_value_name() {
    use crate::semantic_db::types::TypeCandidate;
    let mut ws = VirtualWorkspace::new();
    let file=ws.def("---@generic T\n---@param outer_value T\nlocal function outer(outer_value)\n---@generic U\nlocal function inner()\nlocal C={}\n---@type T\nC.item=outer_value\nend\nend");
    let model = ws.analysis.semantic_model(file);
    let member = model
        .file_facts()
        .unwrap()
        .members
        .iter()
        .find(|m| m.key == crate::LuaMemberKey::from("item"))
        .unwrap();
    let shell = model
        .analysis()
        .member_type(file, member.id.clone())
        .unwrap();
    expect_true!(
        shell
            .candidates
            .iter()
            .any(|c| matches!(c,TypeCandidate::Generic(name) if name=="T")),
        "{shell:?}"
    );
}

#[gtest]
fn annotated_runtime_member_function_generic_shadows_class_parameter() {
    let mut ws = VirtualWorkspace::new();
    let file=ws.def("---@class Box<T:integer,U:string>\nlocal C={}\n---@type fun<U>(value:U):U\nC.item=function(value) return value end");
    let model = ws.analysis.semantic_model(file);
    let member = model
        .file_facts()
        .unwrap()
        .members
        .iter()
        .find(|m| m.key == crate::LuaMemberKey::from("item"))
        .unwrap();
    let Some(LuaType::DocFunction(fun)) = model.type_of_member(&member.id) else {
        panic!("expected function contract")
    };
    for ty in [fun.get_params()[0].1.as_ref().unwrap(), fun.get_ret()] {
        expect_true!(
            matches!(ty,LuaType::TplRef(t) if t.get_name()=="U" && t.get_tpl_id().is_func() && t.get_constraint().is_none()),
            "{ty:?}"
        );
    }
}

#[gtest]
fn annotated_runtime_member_generic_shell_does_not_capture_shadowed_class_name() {
    use crate::semantic_db::types::TypeCandidate;
    for shadowed in [false, true] {
        let assignment = "---@type U\nC.item=nil\n";
        let body = if shadowed {
            format!("---@generic U:boolean\nlocal function set_item()\n{assignment}end")
        } else {
            format!("---@generic U:boolean\nlocal function unrelated() end\n{assignment}")
        };
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(&format!(
            "---@class Box<T:integer,U:string>\nlocal C={{}}\n{body}"
        ));
        let foreign = ws.def("local unrelated=true");
        for model_file in [file, foreign] {
            let model = ws.analysis.semantic_model(model_file);
            let member = model
                .file_facts_of(file)
                .unwrap()
                .members
                .iter()
                .find(|m| m.key == crate::LuaMemberKey::from("item"))
                .unwrap();
            let shell = model
                .analysis()
                .member_type(file, member.id.clone())
                .unwrap();
            expect_that!(
                shell
                    .candidates
                    .iter()
                    .any(|c| matches!(c,TypeCandidate::Generic(name) if name=="U")),
                eq(!shadowed),
                "shadowed={shadowed}: {shell:?}"
            );
            let names = model.member_generic_names(file, &member.id);
            let bound = model.analysis().type_shell_lua_in(file, &shell, &names);
            let rich = model.type_of_member(&member.id).unwrap();
            for ty in [&bound, &rich] {
                if shadowed {
                    expect_true!(
                        matches!(ty,LuaType::Ref(id) if id.get_name()=="U"),
                        "mixed lexical binder remains unresolved: {ty:?}"
                    );
                } else {
                    expect_true!(
                        matches!(ty,LuaType::TplRef(t) if t.get_name()=="U" && t.get_tpl_id()==crate::GenericTplId::Type(1)),
                        "{ty:?}"
                    );
                }
            }
        }
    }
}
