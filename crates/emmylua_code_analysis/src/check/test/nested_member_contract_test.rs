use crate::{DiagnosticCode, VirtualWorkspace};
use emmylua_parser::{LuaAstNode, LuaIndexExpr};

#[test]
fn nested_member_reads_preserve_narrowed_receiver_contracts() {
    for (field, receiver, expected) in [
        ("Count", "Record", "Count"),
        ("Count?", "Record", "Count?"),
        ("T", "Record<Count>", "Count"),
        ("T?", "Record<Count>", "Count?"),
    ] {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(&format!(
            r#"
            ---@alias Count integer
            ---@class Record<T>
            ---@field count {field}
            "#
        ));
        let file = ws.def(&format!(
            r#"
            ---@param parent {{item:{receiver}?}}
            local function update(parent)
                ---@cast parent.item {receiver}
                parent.item.count = parent.item.count or 0
            end
            "#
        ));
        let expected = ws.ty(expected);
        let model = ws.analysis.semantic_model(file);
        let rhs = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .filter(|index| index.syntax().text() == "parent.item.count")
            .last()
            .unwrap();
        assert_eq!(
            model.type_of_expr_at(rhs.get_syntax_id(), rhs.get_position()),
            expected,
            "field={field}, receiver={receiver}"
        );
    }
}

#[test]
fn nested_member_contracts_preserve_optional_receivers() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def("---@class Record\n---@field count integer\nlocal Record = {}");
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::NeedCheckNil,
        r#"
        ---@param parent {item:Record}?
        local function update(parent)
            parent.item.count = parent.item.count + 1
        end
        "#
    ));
}

#[test]
fn nested_member_contracts_keep_explicit_non_nil_guards() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def("---@class Record\n---@field count integer?");
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::NeedCheckNil,
        r#"
        ---@param parent {item:Record}
        local function read(parent)
            if parent.item.count then
                return parent.item.count + 1
            end
        end
    "#
    ));
}
