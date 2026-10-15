#[cfg(test)]
mod test {
    use crate::VirtualWorkspace;

    #[test]
    fn test_inherit_type() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
            ---@class A
            ---@field aaa? fun(a: string)

            local a ---@type A

            function a.aaa(a)
                d = a
            end
            "#,
        );

        let string_ty = ws.expr_ty("d");
        let expected = ws.ty("string");
        assert_eq!(string_ty, expected);
    }
}

#[test]
fn inherited_dictionary_keeps_generic_value_type() {
    let mut ws = crate::VirtualWorkspace::new();
    ws.def(
        r#"
---@alias uint32 integer
---@class Player
---@field name string
---@class CustomTable<K, V>: userdata, table<K, V>
---@type CustomTable<uint32|string, Player>
players = {}
---@type uint32
key = 1
---@type uint32?
optional_key = nil
"#,
    );
    for expr in ["players[key]", "players[1]", "players['alice']"] {
        let actual = ws.expr_ty(expr);
        let expected = ws.ty("Player");
        assert_eq!(actual, expected, "{expr}");
    }
    assert_eq!(ws.expr_ty("players[optional_key]"), ws.ty("Player?"));
    assert!(ws.has_no_diagnostic(
        crate::DiagnosticCode::UndefinedField,
        "local p = players[key]\nlocal q = players[optional_key]"
    ));
}

#[test]
fn inherited_dictionary_substitutes_nested_default_and_alias_arguments() {
    let mut ws = crate::VirtualWorkspace::new();
    ws.def(
        r#"
---@class Person
---@field name string
---@class Mapping<K, V>: table<K, V>
---@class NamedMap<V>: Mapping<string, V>
---@class People<T = Person>: NamedMap<T>
---@alias PeopleAlias People
---@type PeopleAlias
people = {}
---@class Reordered<V,K>: Mapping<K,V>
---@type Reordered<Person, integer>
reordered = {}
---@type string
name = 'a'
---@type integer?
optional = nil
"#,
    );
    assert_eq!(ws.expr_ty("people[name]"), ws.ty("Person"));
    assert_eq!(ws.expr_ty("reordered[optional]"), ws.ty("Person?"));
}

#[test]
fn inherited_dictionary_preserves_declared_members_and_rejects_bad_keys() {
    let mut ws = crate::VirtualWorkspace::new();
    ws.def(
        r#"
---@class Person
---@class People: table<string, Person>
---@field count integer
---@type People
people = {}
---@class NumberMap: table<integer, Person>
---@type NumberMap
numbers = {}
---@type boolean
invalid = true
"#,
    );
    assert_eq!(ws.expr_ty("people.count"), ws.ty("integer"));
    assert_eq!(ws.expr_ty("people['count']"), ws.ty("integer"));
    assert!(!ws.has_no_diagnostic(
        crate::DiagnosticCode::UndefinedField,
        "local p = numbers[invalid]"
    ));
    assert!(!ws.has_no_diagnostic(
        crate::DiagnosticCode::UndefinedField,
        "local p = numbers.missing"
    ));
    assert!(!ws.has_no_diagnostic(
        crate::DiagnosticCode::AssignTypeMismatch,
        "---@type string\nlocal wrong = people['alice']"
    ));
}

#[test]
fn inherited_dictionary_bounds_recursive_hierarchies() {
    let mut ws = crate::VirtualWorkspace::new();
    ws.def(
        r#"
---@class A: B
---@class B: A
---@class Growing<T>: Growing<T[]>
---@type A
cycle = {}
---@type Growing<string>
growing = {}
---@type integer
index = 1
"#,
    );
    assert_eq!(ws.expr_ty("cycle[index]"), crate::LuaType::Unknown);
    assert_eq!(ws.expr_ty("growing[index]"), crate::LuaType::Unknown);
}
