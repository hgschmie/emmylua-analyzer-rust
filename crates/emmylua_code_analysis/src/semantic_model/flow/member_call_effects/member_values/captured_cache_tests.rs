use super::*;
use emmylua_parser::LuaIndexExpr;

#[test]
fn captured_member_mutability_reuses_completed_scans_and_refreshes_after_edits() {
    let mut ws = crate::VirtualWorkspace::new();
    let original = "local methods={run=function() end};local observed=methods.run";
    let file = ws.def_file("captured.lua", original);
    for (replacement, expected) in [
        (None, false),
        (
            Some(
                "local methods={run=function() end};methods.run=function() end;local observed=methods.run",
            ),
            true,
        ),
    ] {
        if let Some(source) = replacement {
            assert_eq!(ws.def_file("captured.lua", source), file);
        }
        let model = ws.analysis.semantic_model(file);
        let index = model
            .chunk()
            .unwrap()
            .descendants::<LuaIndexExpr>()
            .last()
            .unwrap();
        let path = member_access_path(&model, &LuaExpr::IndexExpr(index)).unwrap();
        CAPTURED_MEMBER_SCANS.with(|count| count.set(0));
        for _ in 0..16 {
            assert_eq!(mutable_captured_member(&model, &path), expected);
        }
        assert_eq!(CAPTURED_MEMBER_SCANS.with(|count| count.get()), 1);
    }
}
