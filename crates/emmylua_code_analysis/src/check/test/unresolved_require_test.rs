#[cfg(test)]
mod tests {
    use crate::{DiagnosticCode, EmmyrcWorkspaceModuleMap, VirtualWorkspace};

    #[test]
    fn test_unresolved_require() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnresolvedRequire,
            r#"
            local a = require("missing.module")
            "#,
        ));
    }

    #[test]
    fn test_resolved_require() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "test.lua",
            r#"
            local M = {}
            return M
            "#,
        );

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnresolvedRequire,
            r#"
            local a = require("test")
            "#,
        ));
    }

    #[test]
    fn test_resolved_require_without_known_export() {
        for source in [
            "-- empty module",
            "Registered = true",
            "return UnknownExport",
        ] {
            let mut ws = VirtualWorkspace::new();
            ws.def_file("script/register.lua", source);
            assert!(
                ws.has_no_diagnostic(
                    DiagnosticCode::UnresolvedRequire,
                    r#"require("script.register")"#,
                ),
                "{source}"
            );
        }
    }

    #[test]
    fn test_hidden_module_is_resolved_but_not_visible() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file("hidden.lua", "---@meta no-require\nreturn {}");
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnresolvedRequire, r#"require("hidden")"#,));
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RequireModuleNotVisible,
            r#"require("hidden")"#,
        ));
    }

    #[test]
    fn test_non_literal_require() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnresolvedRequire,
            r#"
            local function module_name()
                return "missing.module"
            end
            local a = require(module_name)
            "#,
        ));
    }

    #[test]
    fn test_factorio_require_paths_with_module_map() {
        let mut ws = VirtualWorkspace::new();
        let mut emmyrc = ws.get_emmyrc();
        emmyrc.workspace.module_map = vec![
            EmmyrcWorkspaceModuleMap {
                pattern: "^__(.*)__(.*)$".to_string(),
                replace: "$1$2".to_string(),
            },
            EmmyrcWorkspaceModuleMap {
                pattern: "^(.*)\\.lua$".to_string(),
                replace: "$1".to_string(),
            },
        ];
        ws.update_emmyrc(emmyrc);
        ws.def_file(
            "signalstrings/signalstrings.lua",
            r#"
            return {}
            "#,
        );
        ws.def_file("signalstrings/register.lua", "Registered = true");

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnresolvedRequire,
            r#"
            local a = require("__signalstrings__/signalstrings.lua")
            local b = require("__signalstrings__.signalstrings")
            local c = require("__signalstrings__/signalstrings")
            require("__signalstrings__/register")
            "#,
        ));
    }
}
