use crate::{DiagnosticCode, VirtualWorkspace};

fn workspace() -> VirtualWorkspace {
    let mut ws = VirtualWorkspace::new();
    ws.def_files(vec![
        (
            "global.lua",
            r#"
            ---@class Source
            ---@field unrelated boolean
        "#,
        ),
        (
            "source.lua",
            r#"
            ---@namespace graphics
            ---@class Source
            ---@field position? number[]
            ---@field size? number[]
            ---@field filename string
        "#,
        ),
        (
            "parameters.lua",
            r#"
            ---@namespace graphics
            ---@class Parameters: Source
            ---@field flags? string[]
        "#,
        ),
        (
            "sprite.lua",
            r#"
            ---@namespace graphics
            ---@class Sprite: Parameters
            ---@field dice? integer
        "#,
        ),
    ]);
    ws
}

#[test]
fn namespaced_constructor_accepts_inherited_optional_members() {
    for code in [
        DiagnosticCode::InjectField,
        DiagnosticCode::MissingFields,
        DiagnosticCode::AssignTypeMismatch,
    ] {
        let mut ws = workspace();
        assert!(ws.has_no_diagnostic(code, r#"
            ---@namespace consumer
            ---@type graphics.Sprite
            local sprite = {filename = 'image.png', position = {1, 2}, size = {4, 4}, flags = {'icon'}}
            return sprite
        "#), "{code:?}");
    }
}

#[test]
fn namespaced_constructor_checks_inherited_values() {
    let mut ws = workspace();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@type graphics.Sprite
        local sprite = {filename = 'image.png', position = false}
        return sprite
    "#
    ));
}

#[test]
fn namespaced_constructor_checks_inherited_required_members() {
    let mut ws = workspace();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::MissingFields,
        r#"
        ---@type graphics.Sprite
        local sprite = {size = {4, 4}}
        return sprite
    "#
    ));
}

#[test]
fn namespaced_constructor_still_rejects_unknown_members() {
    let mut ws = workspace();
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::InjectField,
        r#"
        ---@type graphics.Sprite
        local sprite = {filename = 'image.png', unexpected = true}
        return sprite
    "#
    ));
}

#[test]
fn namespaced_constructor_preserves_child_optional_override() {
    let mut ws = workspace();
    ws.def_file(
        "layered.lua",
        r#"
        ---@namespace graphics
        ---@class Layered: Sprite
        ---@field filename? string
    "#,
    );
    for code in [
        DiagnosticCode::MissingFields,
        DiagnosticCode::AssignTypeMismatch,
    ] {
        assert!(
            ws.has_no_diagnostic(
                code,
                r#"
            ---@type graphics.Layered
            local sprite = {size = {4, 4}}
            return sprite
        "#
            ),
            "{code:?}"
        );
    }
}

#[test]
fn namespaced_constructor_checks_inherited_index_signatures() {
    for (value, valid) in [("'ok'", true), ("false", false)] {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "dictionary.lua",
            r#"
            ---@namespace storage
            ---@class Entries
            ---@field [integer] string
            ---@class Dictionary: Entries
        "#,
        );
        let source = format!(
            r#"
            ---@type storage.Dictionary
            local entries = {{[1] = {value}}}
            return entries
        "#
        );
        assert!(ws.has_no_diagnostic(DiagnosticCode::InjectField, &source));
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::AssignTypeMismatch, &source),
            valid
        );
    }
}

#[test]
fn namespaced_constructor_resolves_qualified_parent_and_cycles() {
    let mut ws = workspace();
    ws.def_file(
        "qualified.lua",
        r#"
        ---@namespace other
        ---@class Image: graphics.Sprite
        ---@class CycleA: CycleB
        ---@field first? string
        ---@class CycleB: CycleA
        ---@field second? integer
    "#,
    );
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::InjectField,
        r#"
        ---@type other.Image
        local image = {filename = 'image.png', flags = {'icon'}}
        ---@type other.CycleA
        local cycle = {first = 'value', second = 2}
        return image, cycle
    "#
    ));
}
