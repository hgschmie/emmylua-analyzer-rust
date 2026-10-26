use emmylua_parser::{LuaAstNode, LuaAstToken, LuaLocalName};
use tokio_util::sync::CancellationToken;

use crate::{DiagnosticCode, LuaType, LuaTypeDeclId, VirtualWorkspace};

fn assert_declaration_types(source: &str, expectations: &[(&str, &str)]) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let expected = expectations
        .iter()
        .map(|(_, ty)| ws.ty(ty))
        .collect::<Vec<_>>();
    let file = ws.def(source);
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    for ((name, _), expected) in expectations.iter().zip(expected) {
        let local = chunk
            .descendants::<LuaLocalName>()
            .find(|local| local.get_name_token().unwrap().get_name_text() == *name)
            .unwrap();
        let decl = model
            .decl_by_offset(local.get_name_token().unwrap().get_position())
            .unwrap();
        let fallback = model.analysis().decl_type_lua(file, decl.clone()).unwrap();
        let rich = model.type_of_decl(&decl).unwrap();
        assert_eq!(fallback, expected, "fallback type of {name}");
        assert_eq!(rich, expected, "rich type of {name}");
    }
}

#[test]
fn declaration_fallback_selects_method_result_slots() {
    assert_declaration_types(
        r#"
---@class SlotManager
local Manager = {}

---@return string?
---@return integer?
function Manager:results() return nil end

function Manager:dispatch()
    local first, second = self:results()
end
"#,
        &[("first", "string?"), ("second", "integer?")],
    );
}

#[test]
fn declaration_fallback_does_not_add_errors_after_invalid_call() {
    let source = r#"
---@class SlotManager
local Manager = {}

---@param value integer
---@return string?
---@return (fun())?
function Manager:results(value) return nil end

---@return string?
function Manager:single() return nil end

---@param name string
function Manager:consume(name) end

function Manager:dispatch()
    local name, callback = self:results("bad")
    if name and callback then
        callback()
    else
        name = self:single()
        if name then self:consume(name) end
    end
end
"#;
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let file = ws.def(source);
    let diagnostics = ws
        .analysis
        .diagnose_file(file, CancellationToken::new())
        .unwrap();
    let relevant = diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(diagnostic.code.as_ref(), Some(lsp_types::NumberOrString::String(code))
                if code == "param-type-mismatch" || code == "call-non-callable")
        })
        .collect::<Vec<_>>();
    assert_eq!(relevant.len(), 1, "{relevant:#?}");
    assert!(relevant[0].message.contains("expected `integer`"));
    let bad_line = source
        .lines()
        .position(|line| line.contains("\"bad\""))
        .unwrap();
    assert_eq!(relevant[0].range.start.line as usize, bad_line);
}

#[test]
fn declaration_fallback_preserves_explicit_annotations() {
    assert_declaration_types(
        r#"
---@return integer, integer
local function results() return 1, 2 end

---@type string, boolean
local first, second = results()
"#,
        &[("first", "string"), ("second", "boolean")],
    );
}

#[test]
fn declaration_fallback_preserves_tuple_values() {
    assert_declaration_types(
        r#"
---@return [string, integer]
local function result() end
local first, extra = result()
"#,
        &[("first", "[string, integer]"), ("extra", "any")],
    );
}

#[test]
fn declaration_fallback_preserves_unknown_results() {
    let mut ws = VirtualWorkspace::new();
    let file = ws.def("local first, second = unavailable()");
    let model = ws.analysis.semantic_model(file);
    for local in model.chunk().unwrap().descendants::<LuaLocalName>() {
        let decl = model
            .decl_by_offset(local.get_name_token().unwrap().get_position())
            .unwrap();
        assert_eq!(
            model.analysis().decl_type_lua(file, decl.clone()),
            Some(LuaType::Unknown)
        );
        assert_eq!(model.type_of_decl(&decl), Some(LuaType::Unknown));
    }
}

#[test]
fn declaration_fallback_finite_result_tail_is_nil() {
    assert_declaration_types(
        r#"
---@return integer, string
local function results() return 1, "ok" end
local first, second, extra = results()
"#,
        &[("first", "integer"), ("second", "string"), ("extra", "nil")],
    );
}

#[test]
fn declaration_fallback_preserves_variadic_tail() {
    assert_declaration_types(
        r#"
---@return string, integer...
local function results() end
local first, second, third, fourth = results()
"#,
        &[
            ("first", "string"),
            ("second", "integer"),
            ("third", "integer"),
            ("fourth", "integer"),
        ],
    );
}

#[test]
fn declaration_fallback_parentheses_keep_one_result() {
    assert_declaration_types(
        r#"
---@return string, integer
local function results() end
local first, extra = (results())
"#,
        &[("first", "string"), ("extra", "any")],
    );
}

#[test]
fn declaration_fallback_preserves_mixed_initializer_slots() {
    assert_declaration_types(
        r#"
---@type boolean
local marker
---@return string, integer
local function results() end
local first, second, third = marker, results()
"#,
        &[
            ("first", "boolean"),
            ("second", "string"),
            ("third", "integer"),
        ],
    );
}

#[test]
fn declaration_fallback_preserves_explicit_call_casts() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let source = r#"
---@return integer, string
local function results() return 1, "ok" end
local callback = results() --[[@as fun()]]
callback()
"#;
    assert!(ws.has_no_diagnostic(DiagnosticCode::CallNonCallable, source));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::CallNonCallable,
        &source.replace("callback()", "---@cast callback string\ncallback()"),
    ));
}

#[test]
fn declaration_fallback_preserves_module_annotations() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let module = ws.def_file("a.lua", "return { value = 1 }");
    let file = ws.def(
        r#"
---@return integer, string
local function results() return 1, "ok" end
---@module "a"
local first, second = results()
"#,
    );
    let model = ws.analysis.semantic_model(file);
    for local in model.chunk().unwrap().descendants::<LuaLocalName>() {
        if !matches!(
            local.get_name_token().unwrap().get_name_text(),
            "first" | "second"
        ) {
            continue;
        }
        let decl = model
            .decl_by_offset(local.get_name_token().unwrap().get_position())
            .unwrap();
        let fallback = model.analysis().decl_type_lua(file, decl.clone());
        let rich = model.type_of_decl(&decl);
        assert_eq!(fallback, Some(LuaType::ModuleRef(module)));
        assert_eq!(rich, Some(LuaType::ModuleRef(module)));
    }
}

fn assert_type_metadata_preserved(kind: &str) {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let source = format!(
        r#"
---@return table
local function seed() return {{}} end
---@{kind} SlotMetadata
local first, second = seed()
"#
    );
    let file = ws.def(&source);
    let model = ws.analysis.semantic_model(file);
    let mut first_fallback = None;
    for local in model.chunk().unwrap().descendants::<LuaLocalName>() {
        if !matches!(
            local.get_name_token().unwrap().get_name_text(),
            "first" | "second"
        ) {
            continue;
        }
        let decl = model
            .decl_by_offset(local.get_name_token().unwrap().get_position())
            .unwrap();
        let fallback = model.analysis().decl_type_lua(file, decl.clone()).unwrap();
        assert!(!fallback.is_any() && !fallback.is_unknown());
        if let Some(first) = &first_fallback {
            assert_eq!(&fallback, first);
        } else {
            first_fallback = Some(fallback);
        }
        assert_eq!(
            model.type_of_decl(&decl),
            Some(LuaType::Ref(LuaTypeDeclId::global("SlotMetadata")))
        );
    }
}

#[test]
fn declaration_fallback_preserves_class_annotations() {
    assert_type_metadata_preserved("class");
}

#[test]
fn declaration_fallback_preserves_enum_annotations() {
    assert_type_metadata_preserved("enum");
}
