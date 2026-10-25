#[cfg(test)]
mod test {
    use hashbrown::HashSet;
    use std::rc::Rc;

    use crate::{LuaType, LuaUnionType, VirtualWorkspace};

    #[test]
    fn test_ipairs_array_and_integer_dictionary_slots() {
        for container in ["Foo[]", "table<integer, Foo>"] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(&format!(
                r#"
                ---@class Foo
                ---@field name string
                ---@param items {container}?
                local function visit(items)
                    if not items then return end
                    for key, value in ipairs(items) do
                        iteration_key = key
                        iteration_value = value
                    end
                end
            "#
            ));
            assert_eq!(ws.expr_ty("iteration_key"), LuaType::Integer, "{container}");
            assert_eq!(ws.expr_ty("iteration_value"), ws.ty("Foo"), "{container}");
        }
    }

    #[test]
    fn test_standard_iterator_factory_return_triples() {
        for iterator in ["pairs", "ipairs"] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(&format!(
                r#"
                ---@class Foo
                ---@field name string
                ---@type Foo[]
                local items = {{}}
                iterator_function, iterator_state, iterator_control = {iterator}(items)
            "#
            ));
            let LuaType::DocFunction(fun) = ws.expr_ty("iterator_function") else {
                panic!("{iterator} should return an iterator function");
            };
            assert_eq!(
                fun.get_ret().get_result_slot_type(0),
                Some(LuaType::Integer)
            );
            assert_eq!(fun.get_ret().get_result_slot_type(1), Some(ws.ty("Foo")));
            let state = ws.expr_ty("iterator_state");
            assert!(
                !matches!(state, LuaType::Unknown | LuaType::Any | LuaType::Nil),
                "{iterator}: {state:?}"
            );
            assert_eq!(
                ws.expr_ty("iterator_control"),
                if iterator == "pairs" {
                    LuaType::Nil
                } else {
                    LuaType::Integer
                }
            );
        }
    }

    #[test]
    fn test_pairs_union_dictionary_and_array() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            ---@class Player
            ---@field valid boolean
            ---@param players table<string, Player>|Player[]
            local function visit(players)
                for key, value in pairs(players) do
                    iteration_key = key
                    iteration_value = value
                end
            end
        "#,
        );
        assert_eq!(ws.expr_ty("iteration_value"), ws.ty("Player"));
        assert_eq!(ws.expr_ty("iteration_key"), ws.ty("string|integer"));
    }

    #[test]
    fn test_pairs_inherited_generic_dictionary() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            ---@class Player
            ---@field valid boolean
            ---@class CustomTable<K, V>: userdata, table<K, V>
            ---@param players CustomTable<string|integer, Player>
            local function visit(players)
                for key, value in pairs(players) do
                    iteration_key = key
                    iteration_value = value
                end
            end
        "#,
        );
        assert_eq!(ws.expr_ty("iteration_value"), ws.ty("Player"));
        assert_eq!(ws.expr_ty("iteration_key"), ws.ty("string|integer"));
    }

    #[test]
    fn test_pairs_union_member_collections() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            ---@class Player
            ---@field valid boolean
            ---@class CustomTable<K, V>: userdata, table<K, V>
            ---@class Game
            ---@field players CustomTable<string|integer, Player>
            ---@class Force
            ---@field players Player[]
            ---@param target Game|Force
            local function visit(target)
                for key, value in pairs(target.players) do
                    iteration_key = key
                    iteration_value = value
                end
            end
        "#,
        );
        assert_eq!(ws.expr_ty("iteration_value"), ws.ty("Player"));
        assert_eq!(ws.expr_ty("iteration_key"), ws.ty("string|integer"));
    }

    #[test]
    fn test_pairs_union_factory_signature() {
        for collection in [
            "table<string, Player>|Player[]",
            "Player[]|CustomTable<string, Player>",
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(&format!(
                r#"
                ---@class Player
                ---@field valid boolean
                ---@class CustomTable<K, V>: userdata, table<K, V>
                ---@param players {collection}
                local function visit(players)
                    iterator_function = pairs(players)
                end
            "#
            ));
            let LuaType::DocFunction(fun) = ws.expr_ty("iterator_function") else {
                panic!("pairs should return an iterator function");
            };
            assert_eq!(
                fun.get_ret().get_result_slot_type(0),
                Some(ws.ty("string|integer")),
                "{collection}"
            );
            assert_eq!(
                fun.get_ret().get_result_slot_type(1),
                Some(ws.ty("Player")),
                "{collection}"
            );
        }
    }

    #[test]
    fn test_pairs_union_preserves_distinct_values() {
        for collection in [
            "table<string, string>|integer[]",
            "integer[]|table<string, string>",
        ] {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.def(&format!(
                r#"
                ---@param values {collection}
                local function visit(values)
                    for key, value in pairs(values) do
                        iteration_key = key
                        iteration_value = value
                    end
                end
            "#
            ));
            assert_eq!(ws.expr_ty("iteration_key"), ws.ty("string|integer"));
            assert_eq!(ws.expr_ty("iteration_value"), ws.ty("string|integer"));
        }
    }

    #[test]
    fn test_pairs_inherited_generic_alias_and_guard() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            ---@class CustomTable<K, V>: userdata, table<K, V>
            ---@class Derived<T>: CustomTable<string, T>
            ---@alias Collection<T> Derived<T>|T[]
            ---@param values Collection<boolean>?
            local function visit(values)
                if not values then return end
                for key, value in pairs(values) do
                    iteration_key = key
                    iteration_value = value
                end
            end
        "#,
        );
        assert_eq!(ws.expr_ty("iteration_key"), ws.ty("string|integer"));
        assert_eq!(ws.expr_ty("iteration_value"), LuaType::Boolean);
    }

    #[test]
    fn test_collection_union_keeps_unresolved_alternatives() {
        let mut ws = VirtualWorkspace::new();
        let file = ws.def(
            r#"
            ---@class Recursive<T>: Recursive<T>
        "#,
        );
        for ty in [
            "string[]|nil",
            "string[]|string",
            "string[]|unknown",
            "string[]|Recursive<string>",
        ] {
            let ty = ws.ty(ty);
            assert!(
                ws.analysis
                    .semantic_model(file)
                    .collection_key_value_types(&ty)
                    .is_none(),
                "{ty:?}"
            );
        }
    }

    #[test]
    fn test_collection_union_generic_call() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@generic K, V
            ---@param values table<K, V>
            ---@return K, V
            local function first(values) end
            ---@param values table<string, boolean>|string[]
            local function visit(values)
                first_key, first_value = first(values)
            end
        "#,
        );
        assert_eq!(ws.expr_ty("first_key"), ws.ty("string|integer"));
        assert_eq!(ws.expr_ty("first_value"), ws.ty("boolean|string"));
    }

    #[test]
    fn test_closure_param_infer() {
        let mut ws = VirtualWorkspace::new();

        ws.def(
            r#"
        ---@alias foo (fun(tbl: any): (number, string))

        ---@type foo
        local b = {}

        for k3, v3 in b do
            k1 = k3
            v1 = v3
        end


        ---@class bar
        ---@overload fun(tbl: any): (number, string)

        ---@type bar
        local c = {}

        for k4, v4 in c do
            k2 = k4
            v2 = v4
        end
        "#,
        );

        assert_eq!(ws.expr_ty("k1"), LuaType::Number);
        assert_eq!(ws.expr_ty("v1"), LuaType::String);
        assert_eq!(ws.expr_ty("k2"), LuaType::Number);
        assert_eq!(ws.expr_ty("v2"), LuaType::String);
    }

    #[test]
    fn test_issue_227() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
        local a --- @type any

        for k in pairs(a) do
            -- k should be any not integer
            d = k
        end
        "#,
        );

        assert_eq!(ws.expr_ty("d"), LuaType::Any);
    }

    #[test]
    fn test_issue_321() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
        ---@return fun():string?
        local function test(...) end

        for k in test() do
            -- k can't be nil
            d = k
        end
        "#,
        );

        assert_eq!(ws.expr_ty("d"), LuaType::String);
    }

    #[test]
    fn test_issue_490() {
        let mut ws = VirtualWorkspace::new();

        ws.def(
            r#"
        ---@generic T: table, K, V
        ---@param t T
        ---@return fun(table: table<K, V>, index?: K):K, V
        ---@return T
        local function spairs(t) end

        --- @type table<string, integer>
        local t = { a = 1, b = 2, c = 3 }
        for name, value in spairs(t) do
            a = name
            b = value
        end
        "#,
        );

        let a = ws.expr_ty("a");
        let b = ws.expr_ty("b");
        assert_eq!(a, LuaType::String);
        assert_eq!(b, LuaType::Integer);
    }

    #[test]
    fn test_enum_key_pairs() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            --- @enum Severity
            local severity = {
                ERROR = 1,
                WARN = 2,
                INFO = 3,
                HINT = 4,
            }

            local severities = {
                [severity.ERROR] = 1,
                [severity.WARN] = 2,
                [severity.INFO] = 3,
                [severity.HINT] = 4,
            }

            for k in pairs(severities) do
                key = k
            end
        "#,
        );

        let key_ty = ws.expr_ty("key");
        let LuaType::Union(union) = key_ty else {
            panic!("expected enum key union, got {:?}", key_ty);
        };
        let set = union.into_set();
        let expected: HashSet<_> = vec![
            LuaType::IntegerConst(1),
            LuaType::IntegerConst(2),
            LuaType::IntegerConst(3),
            LuaType::IntegerConst(4),
        ]
        .into_iter()
        .collect();
        assert_eq!(set, expected);
    }

    #[test]
    fn test_pairs_expr_key_type() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            local key = tostring(1)
            local t = {
                [key] = 1,
            }

            for k in pairs(t) do
                key_out = k
            end
        "#,
        );

        assert_eq!(ws.expr_ty("key_out"), LuaType::String);
    }

    #[test]
    fn test_pairs_value_field_integer_keys() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            local t = { 10, 20, 30 }

            for k, v in pairs(t) do
                key_out = k
                value_out = v
            end
        "#,
        );

        let key_out = ws.expr_ty("key_out");
        let value_out = ws.expr_ty("value_out");
        let LuaType::Union(key_union) = key_out else {
            panic!("expected integer key union, got {:?}", key_out);
        };
        let LuaType::Union(value_union) = value_out else {
            panic!("expected value union, got {:?}", value_out);
        };

        let expected_keys: HashSet<_> = vec![
            LuaType::IntegerConst(1),
            LuaType::IntegerConst(2),
            LuaType::IntegerConst(3),
        ]
        .into_iter()
        .collect();
        let expected_values: HashSet<_> = vec![
            LuaType::DocIntegerConst(10),
            LuaType::DocIntegerConst(20),
            LuaType::DocIntegerConst(30),
        ]
        .into_iter()
        .collect();

        assert_eq!(key_union.into_set(), expected_keys);
        assert_eq!(value_union.into_set(), expected_values);
    }

    #[test]
    fn test_pairs_self_iterator_method_does_not_recurse() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            ---@class Base
            local base = {}
            function base:iter()
                return pairs(self)
            end

            ---@generic T
            ---@param e T
            ---@return T | Base
            function Enum(e)
            end

            local Event = Enum {
                A = 1,
                B = 2,
            }

            for k, v in Event:iter() do
                key_out = k
                value_out = v
            end
        "#,
        );

        let _ = ws.expr_ty("key_out");
        let _ = ws.expr_ty("value_out");
    }

    #[test]
    fn test_issue_291() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            --- @class A
            --- @field [integer] string
            --- @field a boolean
            --- @field b number
            local a

            for _, v in ipairs(a) do
                d = v
            end
        "#,
        );

        assert_eq!(ws.expr_ty("d"), LuaType::String);
    }

    #[test]
    fn test_issue_291_2() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            --- @class A
            --- @field [1] string
            --- @field [2] number
            local a

            for _, v in ipairs(a) do
                d = v
            end
        "#,
        );

        assert_eq!(
            ws.expr_ty("d"),
            LuaType::Union(Rc::new(LuaUnionType::from_vec(vec![
                LuaType::String,
                LuaType::Number
            ]))),
        );
    }
}
