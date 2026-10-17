#[cfg(test)]
mod optional_class_fields {
    use crate::handlers::{hover::hover, test_lib::ProviderVirtualWorkspace};
    use lsp_types::HoverContents;

    fn text(definitions: &str, source: &str) -> String {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(definitions);
        let (content, position) = ProviderVirtualWorkspace::handle_file_content(source).unwrap();
        let file = ws.def(&content);
        let result = hover(&ws.analysis, file, position).unwrap();
        let HoverContents::Markup(markup) = result.contents else {
            panic!("markup required")
        };
        markup.value
    }

    const DEFINITIONS: &str = r#"
        ---@alias uint32 integer
        ---@class Burner
        ---@field fuel string
        ---@class Entity
        ---@field burner? Burner
        ---@field start_index? uint32
        ---@field required Burner
        ---@field already_nullable? Burner?
        ---@field children? Burner[]
        ---@field callback? fun(): integer
    "#;

    fn check_fields(result: &str) {
        for expected in [
            "burner: Burner?,",
            "start_index: uint32?,",
            "required: Burner,",
            "already_nullable: Burner?,",
            "children: Burner[]?,",
        ] {
            assert!(result.contains(expected), "missing {expected}: {result}");
        }
        assert!(!result.contains("??"), "{result}");
    }

    #[test]
    fn optional_fields_in_local_declaration_and_reference() {
        for source in [
            "---@type Entity\nlocal <??>entity = {}",
            "---@type Entity\nlocal entity = {}\nprint(<??>entity)",
        ] {
            let result = text(DEFINITIONS, source);
            check_fields(&result);
            assert!(result.contains("callback: function?,"), "{result}");
        }
    }

    #[test]
    fn optional_fields_in_class_hover() {
        let result = text(DEFINITIONS, "---@type <??>Entity\nlocal entity");
        check_fields(&result);
        assert!(
            result.contains("callback: (fun() -> integer)?,"),
            "{result}"
        );
    }

    #[test]
    fn optional_inherited_field() {
        let definitions = format!(
            "{DEFINITIONS}\n---@class Box\n---@field value? Burner\n---@class BurnerBox: Box"
        );
        for source in [
            "---@type BurnerBox\nlocal <??>box = {}",
            "---@type <??>BurnerBox\nlocal box",
        ] {
            let result = text(&definitions, source);
            assert!(result.contains("value: Burner?,"), "{result}");
        }
    }

    #[test]
    fn class_summary_keeps_declared_optionality_after_field_cast() {
        let result = text(
            DEFINITIONS,
            "---@type Entity\nlocal entity = {}\n---@cast entity.burner -?\nprint(<??>entity)",
        );
        check_fields(&result);
    }
}

#[cfg(test)]
mod tests {
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualHoverResult, check};
    use googletest::prelude::*;

    fn dedent(input: &str) -> String {
        let lines: Vec<&str> = input.lines().collect();
        let mut min_indent = usize::MAX;
        for line in &lines {
            if line.trim().is_empty() {
                continue;
            }
            let indent = line.chars().take_while(|c| *c == ' ').count();
            min_indent = min_indent.min(indent);
        }
        if min_indent == usize::MAX {
            return String::new();
        }
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = if line.len() >= min_indent {
                &line[min_indent..]
            } else {
                line
            };
            out.push_str(trimmed);
            if i + 1 < lines.len() {
                out.push('\n');
            }
        }
        out.trim_start_matches('\n').trim_end().to_string()
    }

    #[gtest]
    fn test_negative_literal_hover_includes_sign_and_range() -> Result<()> {
        use crate::handlers::hover::hover;
        use lsp_types::{HoverContents, Position, Range};

        for (expression, expected) in [
            ("-1", "-1"),
            ("-1.5", "-1.5"),
            ("-0x10", "-16"),
            ("- 12", "-12"),
        ] {
            let mut ws = ProviderVirtualWorkspace::new();
            let source = format!("return {expression}");
            let file_id = ws.def(&source);
            for column in [7, source.len() as u32 - 1] {
                let result = hover(&ws.analysis, file_id, Position::new(0, column)).or_fail()?;
                let HoverContents::Markup(contents) = result.contents else {
                    return fail!("expected Markdown hover");
                };
                check!(verify_eq!(
                    contents.value,
                    format!("```lua\n{expected}\n```")
                ));
                check!(verify_eq!(
                    result.range,
                    Some(Range::new(
                        Position::new(0, 7),
                        Position::new(0, source.len() as u32)
                    ))
                ));
            }
        }
        Ok(())
    }

    #[gtest]
    fn test_negative_literal_hover_in_context() -> Result<()> {
        for (source, expected) in [
            (
                "local stop = {}\nstop.error_code = stop.error_code or stop.errorCode or -<??>1",
                "-1",
            ),
            ("return { value = -<??>1 }", "-1"),
            // Numeric constructor keys keep their member information.
            ("return { [-<??>1] = true }", "(field) [-1]: boolean = true"),
        ] {
            let mut ws = ProviderVirtualWorkspace::new();
            check!(ws.check_hover(
                source,
                VirtualHoverResult {
                    value: format!("```lua\n{expected}\n```")
                }
            ));
        }
        Ok(())
    }

    #[gtest]
    fn test_negative_literal_hover_keeps_operand_boundaries() -> Result<()> {
        for source in [
            "return <??>1",
            "return 3 - <??>1",
            "return -<??>1 ^ 2",
            "return -(<??>1 + 2)",
            "return ~<??>1",
        ] {
            let mut ws = ProviderVirtualWorkspace::new();
            check!(ws.check_hover(
                source,
                VirtualHoverResult {
                    value: "```lua\n1\n```".to_string()
                }
            ));
        }
        Ok(())
    }

    #[gtest]
    fn test_1() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class <??>A
                ---@field a number
                ---@field b string
                ---@field c boolean
            "#,
            VirtualHoverResult {
                value:
                    "```lua\n(class) A {\n    a: number,\n    b: string,\n    c: boolean,\n}\n```"
                        .to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_index_signature() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Foo
                ---@field [integer] string

                ---@type Foo
                local <??>foo
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal foo: Foo {\n    [integer]: string,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_nil_type_key_hidden() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class <??>AAA
                ---@field [true] 1
                ---@field [nil] 2
            "#,
            VirtualHoverResult {
                value: "```lua\n(class) AAA {\n    [true]: integer = 1,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_right_to_left() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        // check!(ws.check_hover(
        //     r#"
        //         ---@class H4
        //         local m = {
        //             x = 1
        //         }

        //         ---@type H4
        //         local m1

        //         m1.x = {}
        //         m1.<??>x = {}
        //     "#,
        //     VirtualHoverResult {
        //         value: "```lua\n(field) x: integer = 1\n```".to_string(),
        //     },
        // ));

        check!(ws.check_hover(
            r#"
                ---@class Node
                ---@field x number
                ---@field right Node?

                ---@return Node
                local function createRBNode()
                end

                ---@type Node
                local node

                if node.right then
                else
                    node.<??>right = createRBNode()
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) right: Node\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                 ---@class Node1
                ---@field x number

                ---@return Node1
                local function createRBNode()
                end

                ---@type Node1?
                local node

                if node then
                else
                    <??>node = createRBNode()
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal node: Node1 {\n    x: number,\n}\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_nil() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class A
                ---@field a? number

                ---@type A
                local a

                local d = a.<??>a
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) a: number?\n```".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_function_infer_return_val() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local function <??>f(a, b)
                    a = 1
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function f(a, b)\n```".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_hover_param_string() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param n string doc
                function foo(<??>n)
                end
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (parameter) n: string
                    ```

                    ---

                    doc
                    "#
                )
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_hover_param_func() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param n fun():boolean doc
                function foo(<??>n)
                end
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (parameter) n: fun() -> boolean
                    ```

                    ---

                    doc
                    "#
                )
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_generic_param_constraint_and_field_description() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Animal
                ---@field name string 名字
                ---@field age integer 年龄

                ---@generic T: Animal
                ---@param animal T
                function checkAnimal(<??>animal)
                    print(animal.age)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) animal: T extends Animal\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class Animal
                ---@field name string 名字
                ---@field age integer 年龄

                ---@generic T: Animal
                ---@param animal T
                function checkAnimal(animal)
                    print(animal.<??>age)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) age: integer\n```\n\n---\n\n年龄".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class Animal
                ---@field name string 名字
                ---@field age integer 年龄

                ---@generic T: Animal
                ---@param animal T?
                function checkAnimal(<??>animal)
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(parameter) animal: T?\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_special_alias_call_type_syntax() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class KeyofHoverShape
                ---@field name string

                ---@type keyof KeyofHoverShape
                local <??>key
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal key: \"name\"\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class IndexHoverShape
                ---@field name string

                ---@type IndexHoverShape["name"]
                local <??>value
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal value: string\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class GenericIndexHoverShape
                ---@field name string

                ---@alias GenericIndexHoverPick<K extends keyof T, T> T[K]

                ---@type GenericIndexHoverPick<"name", GenericIndexHoverShape>
                local <??>value
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal value: string\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@class ExtendsHoverShape
                ---@field name string

                ---@type ExtendsHoverShape extends table and number or string
                local <??>is_table
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal is_table: number\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@alias AB<??>C<K extends keyof T, T> T[K]
            "#,
            VirtualHoverResult {
                value: "```lua\n(alias) ABC<K extends keyof T, T> = T[K]\n```".to_string(),
            },
        ));

        check!(
            ws.check_hover(
                r#"
                ---@alias Co<??>py<T> { readonly [K in keyof T]?: T[K] }
            "#,
                VirtualHoverResult {
                    value: "```lua\n(alias) Copy<T> = { readonly [K in keyof T]?: T[K]; }\n```"
                        .to_string(),
                },
            )
        );

        check!(
            ws.check_hover(
                r#"
                ---@class BoxHoverShape
                ---@field name string

                ---@class BoxHoverShape<??>Box<K extends keyof BoxHoverShape, T>
                ---@field value T
            "#,
                VirtualHoverResult {
                    value:
                        "```lua\n(class) BoxHoverShapeBox<K extends keyof BoxHoverShape, T>\n```"
                            .to_string(),
                },
            )
        );
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_narrowed_function_type() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@param n integer|fun():boolean
                function _G.foo(n)
                    local f = n
                    if type(f) ~= 'function' then
                        f = function()
                            return true
                        end
                    end
                    local _ = <??>f
                end
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (parameter) n: fun() -> boolean
                    ```
                    "#
                ),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_decl_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Buff.AddData
                ---@field pulse? number 心跳周期

                ---@type Buff.AddData
                local data

                data.pu<??>lse
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) pulse: number?\n```\n\n&nbsp;&nbsp;in class `Buff.AddData`\n\n---\n\n心跳周期".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_issue_535() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@type table<string, number>
                local t

                ---@class T1
                local a

                function a:init(p)
                    self._c<??>fg = t[p]
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) _cfg: number\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                ---@type table<string, number>
                local t = {}
                ---@class T2
                local a = {}

                function a:init(p)
                    self._cfg = t[p]
                end

                ---@param p T2
                function fun(p)
                    local x = p._c<??>fg
                end
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) _cfg: number\n```".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_signature_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                -- # A
                local function a<??>bc()
                end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function abc()\n```\n\n---\n\n# A".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn test_class_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---A1
                ---@class AB<??>C
                ---A2
            "#,
            VirtualHoverResult {
                value: "```lua\n(class) ABC\n```\n\n---\n\nA1".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_bound_local_decl_description() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- This is the MyModule documentation.
                --- It should appear when hovering over MyModule.
                --- @class MyModule
                local My<??>Module
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    local MyModule: MyModule
                    ```

                    ---

                    This is the MyModule documentation.
                    It should appear when hovering over MyModule.
                    "#,
                ),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_class_bound_member_description() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- @class MyModule
                local MyModule = {}

                --- This is the SubModule documentation.
                --- It should appear when hovering over SubModule.
                --- @class MyModule.SubModule
                MyModule.Sub<??>Module = {}
            "#,
            VirtualHoverResult {
                value: dedent(
                    r#"
                    ```lua
                    (field) SubModule: MyModule.SubModule
                    ```

                    ---

                    This is the SubModule documentation.
                    It should appear when hovering over SubModule.
                    "#,
                ),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_attribute_hover_uses_arg_types() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class custom_attribute: Attribute
                ---@overload fun(value: string)
                ---@overload fun(value: integer)

                ---@[custom_at<??>tribute(1)]
                local a
            "#,
            VirtualHoverResult {
                value: "```lua\n(class) custom_attribute(value: integer)\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_alias_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@alias Tes<??>Alias
                ---| 'A' # A1
                ---| 'B' # A2
            "#,
            VirtualHoverResult {
                value: "```lua\n(alias) TesAlias = (\"A\"|\"B\")\n    | \"A\" -- A1\n    | \"B\" -- A2\n\n```".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_type_desc() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local export = {
                    ---@type number? activeSub
                    vvv = nil
                }

                export.v<??>vv
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) vvv: number?\n```\n\n---\n\nactiveSub".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_field_key() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
                ---@class ObserverParams
                ---@field next fun() # 测试

                ---@param params fun() | ObserverParams
                function test(params)
                end
            "#,
        );
        check!(ws.check_hover(
            r#"
                test({
                    <??>next = function()
                    end
                })
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) ObserverParams.next()\n```\n\n---\n\n测试".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_field_key_for_generic() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
                ---@class ObserverParams<T>
                ---@field next fun() # 测试

                ---@generic T
                ---@param params fun() | ObserverParams<T>
                function test(params)
                end
            "#,
        );
        check!(ws.check_hover(
            r#"
                test({
                    <??>next = function()
                    end
                })
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) ObserverParams.next()\n```\n\n---\n\n测试".to_string(),
            },
        ));
        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_before_dot_returns_object_info() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
                ---@class Node
                ---@field field number?
                ---@field method fun(self: Node)

                ---@type Node
                node = {}

                function node.method() end
            "#,
        );

        check!(ws.check_hover(
            r#"
                node<??>.field = nil
            "#,
            VirtualHoverResult {
                value: "```lua\n(global) node: Node {\n    field: number?,\n    method: function,\n}\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                node<??>:method()
            "#,
            VirtualHoverResult {
                value: "```lua\n(global) node: Node {\n    field: number?,\n    method: function,\n}\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                node<??>["key"] = "value"
            "#,
            VirtualHoverResult {
                value: "```lua\n(global) node: Node {\n    field: number?,\n    method: function,\n}\n```".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                node["key"<??>] = "value"
            "#,
            VirtualHoverResult {
                value: "\"key\"".to_string(),
            },
        ));

        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_see_tag() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- Description
                ---
                --- @see a.b.c
                local function te<??>st() end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function test()\n```\n\n---\n\nDescription\n\n---\n\n@*see* a.b.c".to_string(),
            },
        ));

        check!(ws.check_hover(
            r#"
                --- Description
                ---
                --- @see a.b.c see description
                local function te<??>st() end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function test()\n```\n\n---\n\nDescription\n\n---\n\n@*see* a.b.c see description".to_string(),
            },
        ));

        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_other_tag() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                --- Description
                ---
                --- @xyz content
                local function te<??>st() end
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal function test()\n```\n\n---\n\nDescription\n\n---\n\n@*xyz* content".to_string(),
            },
        ));

        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_class_with_nil() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def(
            r#"
            ---@class A
            ---@field aAnnotation? string a标签

            ---@class B
            ---@field bAnnotation? string b标签
            "#,
        );
        check!(ws.check_hover(
            r#"
            ---@type A|B|nil
            local defaultOpt = {
                aAnnota<??>tion = "a",
            }
            "#,
            VirtualHoverResult {
                value:
                    "```lua\n(field) aAnnotation: string = \"a\"\n```\n\n---\n\na标签".to_string(),
            },
        ));

        Ok(())
    }

    // M3 batch 2 retirement: depends on the old humanize rendering stack (constant value / alias / keyof / overloaded attributes / narrowing / cross-file declared type projection, etc.); see migration notes §M3.
    #[gtest]
    fn test_hover_right_expr() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                local max = 0
                local key
                if type(key) ~= "number" then
                    return false
                end
                m<??>ax = key
            "#,
            VirtualHoverResult {
                value: "```lua\nlocal max: integer\n```".to_string(),
            },
        ));

        Ok(())
    }

    /// A field of a generic class must be projected through the prefix's type arguments at the use
    /// site: `@field aaa T` + `MyTable<int>` shows `integer`, not the declaration-site `T`.
    #[gtest]
    fn test_hover_generic_field_access_projected() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class MyTable<T>
                ---@field aaa T
                ---@field bbb T

                local c ---@type MyTable<int>

                print(c.<??>aaa)
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) aaa: integer\n```".to_string(),
            },
        ));
        check!(ws.check_hover(
            r#"
                ---@class MyTable<T>
                ---@field aaa T
                ---@field bbb T

                local c ---@type MyTable<int>

                local x = c.<??>bbb
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) bbb: integer\n```".to_string(),
            },
        ));
        Ok(())
    }

    /// Generic arguments keep their parameter positions: `Pair<int, string>.second` is `string`.
    #[gtest]
    fn test_hover_generic_field_access_projected_multi_param() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Pair<T, U>
                ---@field first T
                ---@field second U

                local p ---@type Pair<int, string>

                print(p.<??>second)
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) second: string\n```".to_string(),
            },
        ));
        Ok(())
    }
    #[gtest]
    fn callback_member_hover_does_not_use_enclosing_table_context() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Settings
                ---@field count integer
                ---@alias Handler fun(settings: Settings)
                ---@class Handlers
                ---@field count string
                ---@field change Handler
                ---@type Handlers
                local handlers = {
                    ---@type Handler
                    change = function(settings) settings.<??>count = 1 end,
                }
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) count: integer = 1\n```".to_string(),
            },
        ));
        Ok(())
    }
    #[gtest]
    fn callback_member_read_hover_does_not_use_enclosing_table_context() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Settings
                ---@field count integer
                ---@alias Handler fun(settings: Settings)
                ---@class Handlers
                ---@field count string
                ---@field change Handler
                ---@type Handlers
                local handlers = {
                    ---@type Handler
                    change = function(settings) return settings.<??>count end,
                }
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) count: integer\n```".to_string(),
            },
        ));
        Ok(())
    }

    #[gtest]
    fn value_member_hover_does_not_use_enclosing_table_context() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_hover(
            r#"
                ---@class Settings
                ---@field count integer
                ---@type Settings
                local settings
                ---@class Container
                ---@field count string
                ---@field value integer
                ---@type Container
                local container = { value = settings.<??>count }
            "#,
            VirtualHoverResult {
                value: "```lua\n(field) count: integer\n```".to_string(),
            },
        ));
        Ok(())
    }
}

#[cfg(test)]
mod member_use_site {
    use crate::handlers::{hover::hover, test_lib::ProviderVirtualWorkspace};
    use lsp_types::HoverContents;

    fn text(definitions: &str, source: &str, separate: bool) -> String {
        let mut ws = ProviderVirtualWorkspace::new();
        let source = if separate {
            ws.def(definitions);
            source.to_owned()
        } else {
            format!("{definitions}\n{source}")
        };
        let (content, position) = ProviderVirtualWorkspace::handle_file_content(&source).unwrap();
        let file = ws.def(&content);
        let result = hover(&ws.analysis, file, position).unwrap();
        let HoverContents::Markup(markup) = result.contents else {
            panic!("markup required")
        };
        markup.value
    }

    const DEFINITIONS: &str = r#"
    ---@class Train
    local Train = {
        ---Count stored items.
        ---@param item? string
        ---@return integer
        get_count = function(item) end,
    }
    ---@class Stop
    ---@field train Train? The parked train.
    "#;

    #[test]
    fn narrowed_member_hover_uses_read_type() {
        for separate in [false, true] {
            let result = text(
                DEFINITIONS,
                r#"
            ---@param stop Stop
            local function test(stop)
                if stop.train then
                    print(stop.<??>train)
                end
            end
            "#,
                separate,
            );
            assert!(result.contains("train: Train\n"), "{separate}: {result}");
            assert!(result.contains("The parked train."), "{result}");
        }
    }

    #[test]
    fn narrowed_member_hover_preserves_method_documentation() {
        for separate in [false, true] {
            let result = text(
                DEFINITIONS,
                r#"
            ---@param stop Stop
            local function test(stop)
                if stop.train then
                    stop.train.<??>get_count('iron')
                end
            end
            "#,
                separate,
            );
            assert!(result.contains("get_count"), "{separate}: {result}");
            assert!(result.contains("Count stored items."), "{result}");
            assert!(result.contains("item"), "{result}");
            assert!(result.contains("integer"), "{result}");
        }
    }
    #[test]
    fn member_hover_keeps_unguarded_and_write_contracts() {
        for body in [
            "print(stop.<??>train)",
            "if stop.train then stop.<??>train = nil end",
        ] {
            let source = format!("---@param stop Stop\nlocal function test(stop)\n{body}\nend");
            let result = text(DEFINITIONS, &source, true);
            assert!(result.contains("train: Train?"), "{result}");
        }
        let result = text(
            DEFINITIONS,
            r#"
        ---@param stop Stop
        local function test(stop)
            if not stop.train then print(stop.<??>train) end
        end
        "#,
            true,
        );
        assert!(!result.contains("train: Train\n"), "{result}");
    }

    #[test]
    fn nullable_receiver_identity_does_not_remove_nil_diagnostic() {
        use emmylua_code_analysis::{DiagnosticCode, VirtualWorkspace};
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            &format!(
                r#"
        {DEFINITIONS}
        ---@param stop Stop
        local function test(stop)
            stop.train.get_count('iron')
        end
        "#
            )
        ));
    }

    #[test]
    fn optional_field_marker_does_not_override_casts() {
        let definitions = format!(
            "{}\n---@class Platform\n",
            DEFINITIONS.replace("@field train Train?", "@field train? Train|Platform")
        );
        for separate in [false, true] {
            for body in [
                "---@cast stop.train Train\nprint(stop.<??>train)",
                "---@cast stop.train Train\nlocal callback = function() print(stop.<??>train) end",
            ] {
                let source = format!("---@param stop Stop\nlocal function test(stop)\n{body}\nend");
                let result = text(&definitions, &source, separate);
                assert!(
                    result.contains("train: Train\n"),
                    "{separate} {body}: {result}"
                );
                assert!(result.contains("The parked train."), "{result}");
            }
        }
    }

    #[test]
    fn optional_field_marker_does_not_override_guards() {
        let definitions = DEFINITIONS.replace("@field train Train?", "@field train? Train");
        for separate in [false, true] {
            for body in [
                "if stop.train then print(stop.<??>train) end",
                "assert(stop.train)\nprint(stop.<??>train)",
                "if stop.train ~= nil then print(stop.<??>train) end",
            ] {
                let source = format!("---@param stop Stop\nlocal function test(stop)\n{body}\nend");
                let result = text(&definitions, &source, separate);
                assert!(
                    result.contains("train: Train\n"),
                    "{separate} {body}: {result}"
                );
            }
        }
    }

    #[test]
    fn work_optional_field_flow_boundaries() {
        let definitions = DEFINITIONS.replace("@field train Train?", "@field train? Train");
        for (body, expected) in [
            (
                "if not stop.train then return end\nprint(stop.<??>train)",
                "train: Train\n",
            ),
            (
                "if stop.train then print(other.<??>train) end",
                "train: Train?\n",
            ),
            (
                "if stop.train then stop.train = nil\nprint(stop.<??>train) end",
                "train: nil\n",
            ),
        ] {
            let source = format!(
                "---@param stop Stop\n---@param other Stop\nlocal function test(stop, other)\n{body}\nend"
            );
            for generic in [false, true] {
                let (definitions, source) = if generic {
                    (
                        definitions
                            .replace("class Stop", "class Stop<T>")
                            .replace("train? Train", "train? T"),
                        source.replace(" Stop", " Stop<Train>"),
                    )
                } else {
                    (definitions.clone(), source.clone())
                };
                let result = text(&definitions, &source, true);
                assert!(
                    result.contains(expected),
                    "generic={generic} {body}: {result}"
                );
            }
        }
    }

    #[test]
    fn optional_field_marker_preserves_nullable_reads() {
        let definitions = DEFINITIONS.replace("@field train Train?", "@field train? Train");
        for separate in [false, true] {
            for body in [
                "print(stop.<??>train)",
                "---@cast stop.train Train?\nprint(stop.<??>train)",
                "if stop.train then print(stop.train) end\nprint(stop.<??>train)",
            ] {
                let source = format!("---@param stop Stop\nlocal function test(stop)\n{body}\nend");
                let result = text(&definitions, &source, separate);
                assert!(
                    result.contains("train: Train?\n"),
                    "{separate} {body}: {result}"
                );
            }
        }
    }

    #[test]
    fn optional_field_declaration_retains_marker() {
        let definitions = DEFINITIONS.replace("@field train Train?", "@field <??>train? Train");
        let result = text("", &definitions, false);
        assert!(result.contains("train: Train?\n"), "{result}");
    }

    #[test]
    fn optional_generic_field_uses_read_nullability() {
        let definitions = format!("{DEFINITIONS}\n---@class Box<T>\n---@field value? T\n");
        for (body, expected) in [
            ("print(box.<??>value)", "value: Train?\n"),
            (
                "if box.value then print(box.<??>value) end",
                "value: Train\n",
            ),
        ] {
            let source = format!("---@param box Box<Train>\nlocal function test(box)\n{body}\nend");
            let result = text(&definitions, &source, true);
            assert!(result.contains(expected), "{result}");
        }
    }

    #[test]
    fn optional_field_cast_to_nil_does_not_append_question_mark() {
        let definitions = DEFINITIONS.replace("@field train Train?", "@field train? Train");
        let result = text(
            &definitions,
            "---@param stop Stop\nlocal function test(stop)\n---@cast stop.train nil\nprint(stop.<??>train)\nend",
            true,
        );
        assert!(result.contains("train: nil\n"), "{result}");
    }

    #[test]
    fn optional_callable_field_keeps_signature_rendering() {
        let result = text(
            "---@class Callbacks\n---@field on_count? fun(): integer Count stored items.",
            "---@param callbacks Callbacks\nlocal function test(callbacks)\nlocal callback = callbacks.<??>on_count\nend",
            true,
        );
        assert!(
            result.contains("Callbacks.on_count() -> integer"),
            "{result}"
        );
        assert!(result.contains("Count stored items."), "{result}");
    }
}
