#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_named_table_metatable_export() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "is.lua",
            r#"
            local Is = {}
            setmetatable(Is, {
                ---@param key string
                ---@return fun(value: any): boolean
                __index = function(_, key)
                    return function(value) return value ~= nil end
                end,
            })
            return Is
        "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UndefinedField,
            r#"
            local Is = require('is')
            Is.Function(function() end)
            Is.Table({})
        "#
        ));
        ws.def("Is = require('is')");
        let ty = ws.expr_ty("Is.Table");
        assert_eq!(ws.humanize_type(ty), "fun(value: any) -> boolean");
    }

    #[test]
    fn test_named_table_metatable_alias() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            local root = {}
            local alias = root
            setmetatable(alias, {
                ---@param key string
                ---@return number
                __index = function(_, key) return 42 end,
            })
            result = root
        "#,
        );
        let ty = ws.expr_ty("result.answer");
        assert_eq!(ws.humanize_type(ty), "number");
    }

    #[test]
    fn test_named_table_metatable_dynamic_wrapper() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "predicates.lua",
            r#"
            local Is = { Not = {} }
            local M = {}
            function M.Table(value) return type(value) == 'table' and value end
            function M.Function(value) return type(value) == 'function' end
            setmetatable(Is, {
                __index = function(_, key)
                    return M[key] and function(value) return M[key](value) end or nil
                end,
            })
            setmetatable(Is.Not, {
                __index = function(_, key)
                    return M[key] and function(value) return not M[key](value) end or nil
                end,
            })
            return Is
        "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UndefinedField,
            r#"
            local Is = require('predicates')
            Is.Function(function() end)
            Is.Table({})
        "#
        ));
    }

    #[test]
    fn test_named_table_metatable_reassignment() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            local value = {}
            local original = value
            value = {}
            setmetatable(value, {
                ---@param key string
                ---@return number
                __index = function(_, key) return 42 end,
            })
            old = original
            current = value
        "#,
        );
        let ty = ws.expr_ty("current.answer");
        assert!(ty.is_unknown() || ty.is_nil());
        let ty = ws.expr_ty("old.answer");
        assert!(ty.is_unknown() || ty.is_nil());
    }

    #[test]
    fn test_named_table_metatable_edit_removes_association() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "editable.lua",
            r#"
            local value = {}
            setmetatable(value, {
                ---@param key string
                ---@return number
                __index = function(_, key) return 42 end,
            })
            return value
        "#,
        );
        let ty = ws.expr_ty("require('editable').answer");
        assert_eq!(ws.humanize_type(ty), "number");
        ws.def_file("editable.lua", "local value = {}; return value");
        let ty = ws.expr_ty("require('editable').answer");
        assert!(ty.is_unknown() || ty.is_nil());
    }

    #[test]
    fn test_named_table_metatable_replacement() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            local value = {}
            setmetatable(value, {
                ---@param key string
                ---@return number
                __index = function(_, key) return 42 end,
            })
            setmetatable(value, {
                ---@param key string
                ---@return string
                __index = function(_, key) return 'replacement' end,
            })
            result = value
        "#,
        );
        let ty = ws.expr_ty("result.answer");
        assert_eq!(ws.humanize_type(ty), "string");
    }

    #[test]
    fn test_metatable() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            cmd = setmetatable({}, {
                --- @param command string|string[]
                __call = function (_, command)
                end,

                --- @param command string
                --- @return fun(...:string)
                __index = function(_, command)
                end,
            })
            "#,
        );

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            cmd(1)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            cmd("hello)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            cmd({ "hello", "world" })
        "#
        ));

        let ty = ws.expr_ty("cmd.hihihi");
        let ty_desc = ws.humanize_type(ty);
        assert_eq!(ty_desc, "fun(...: string)");
    }

    #[test]
    fn test_metatable_2() {
        let mut ws = VirtualWorkspace::new();

        ws.def(
            r#"
            ---@class switch
            ---@field map table
            ---@field cachedCases table
            local switchMT = {}
            switchMT.__index = switchMT

            ---@return switch
            local function switch()
                local obj = setmetatable({
                    map = {},
                    cachedCases = {},
                }, switchMT)
                a =  obj
            end
            "#,
        );

        let ty = ws.expr_ty("a");
        assert_eq!(ws.humanize_type(ty), "switch");
    }

    #[test]
    fn test_issue_599() {
        let mut ws = VirtualWorkspace::new();

        ws.def(
            r#"
            ---@class Class.Config
            ---@field abc string
            local ClassConfigMeta = {}

            ---@type table<string, Class.Config>
            local _classConfigMap = {}


            ---@param name string
            ---@return Class.Config
            local function getConfig(name)
                local config = _classConfigMap[name]
                if not config then
                    A = setmetatable({ name = name }, { __index = ClassConfigMeta })
                end
            end
            "#,
        );

        let ty = ws.expr_ty("A");
        assert_eq!(ws.humanize_type(ty), "Class.Config");
    }
}
