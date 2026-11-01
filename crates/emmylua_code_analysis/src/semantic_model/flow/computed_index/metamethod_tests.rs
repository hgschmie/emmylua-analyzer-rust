use super::*;

#[test]
fn custom_access_handlers_do_not_prove_assignment_presence() {
    for handler in ["", "__index", "__newindex"] {
        for anonymous in [false, true] {
            let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
            let extra = if handler.is_empty() {
                String::new()
            } else {
                format!(", {handler}: function")
            };
            let (definitions, owner) = if anonymous {
                (String::new(), format!("{{ [string]: integer[]{extra} }}"))
            } else {
                let field = if handler.is_empty() {
                    String::new()
                } else {
                    format!("---@field {handler} function\n")
                };
                (
                    format!("---@class Rows: table<string, integer[]>\n{field}"),
                    "Rows".to_string(),
                )
            };
            let source = format!(
                "{definitions}---@param rows {owner}\n---@param key string\nlocal function f(rows,key) rows[key]={{1}}; local observed=rows[key] end"
            );
            let file = ws.def(&source);
            let contract = ws.ty(&owner);
            let model = ws.analysis.semantic_model(file);
            let index = model
                .chunk()
                .unwrap()
                .descendants::<LuaIndexExpr>()
                .filter(|index| index.get_text() == "rows[key]")
                .last()
                .unwrap();
            for phase in ["cold", "warm"] {
                assert_eq!(
                    super::super::computed_guard::ordinary_collection(&model, &contract),
                    handler.is_empty(),
                    "classification anonymous={anonymous}, {handler}: {phase}"
                );
                if !anonymous || !handler.is_empty() {
                    assert_eq!(
                        assigned_type(&model, &index, index.get_position()).is_some(),
                        handler.is_empty(),
                        "assignment anonymous={anonymous}, {handler}: {phase}"
                    );
                }
            }
        }
    }
}
