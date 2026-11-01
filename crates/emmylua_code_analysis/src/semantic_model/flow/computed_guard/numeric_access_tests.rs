use super::*;
#[test]
fn exact_numeric_literal_accesses_share_integer_identity() {
    for key in ["1", "1.0", "1ULL", "0x1p0"] {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def(&format!("local rows={{}}; local observed=rows[{key}]"));
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .next()
            .unwrap();
        let found = access(&model, &LuaExpr::IndexExpr(index));
        assert!(
            found.is_some_and(|path| matches!(
                path.parts.as_slice(),
                [Part::Member(LuaMemberKey::Integer(1))]
            )),
            "{key}"
        );
    }
}
