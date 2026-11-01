use super::*;
#[test]
fn computed_assignment_survives_mutation_in_its_own_rhs() {
    for (effect, safe) in [
        ("rows[key]=nil", true),
        ("key=key+1", false),
        ("rows={}", false),
    ] {
        let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
        let file=ws.def(&format!("---@param rows integer[][]\n---@param key integer\nlocal function f(rows,key)\n---@return integer[]\nlocal function make() {effect}; return {{}} end\nrows[key]=make(); local observed=rows[key] end"));
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .filter(|index| index.get_text() == "rows[key]")
            .last()
            .unwrap();
        assert_eq!(
            assigned_type(&model, &index, index.get_position()).is_some(),
            safe,
            "{effect}"
        );
    }
}
