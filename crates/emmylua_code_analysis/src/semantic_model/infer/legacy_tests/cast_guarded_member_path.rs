use crate::{DiagnosticCode, VirtualWorkspace};

const TYPES: &str = r#"
    ---@class Signal
    ---@field name string?
    ---@field kind string?
    local Signal = {}
    ---@return Signal?
    function parse() end
    ---@param value string
    function consume(value) end
"#;

#[test]
fn member_cast_follows_guarded_nullable_receiver() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(TYPES);
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        local item = parse()
        if not item then return end
        ---@cast item.name -?
        ---@cast item.kind -?
        ---@type string
        local name = item.name
        ---@type string
        local kind = item.kind
    "#
    ));
}

#[test]
fn member_cast_does_not_affect_another_instance() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(TYPES);
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@param first Signal
        ---@param second Signal
        local function run(first, second)
            ---@cast first.name -?
            ---@type string
            local name = second.name
        end
    "#
    ));
}

#[test]
fn member_cast_does_not_affect_a_sibling_field() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(TYPES);
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        local item = parse()
        if not item then return end
        ---@cast item.name -?
        ---@type string
        local kind = item.kind
    "#
    ));
}

#[test]
fn guarded_member_cast_survives_unrelated_calls_and_branches() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(TYPES);
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        local item = parse()
        if not item then return end
        ---@cast item.name -?
        if math.random() > 0.5 then print('ready') end
        ---@type string
        local name = item.name
    "#
    ));
}

#[test]
fn replacing_the_receiver_invalidates_older_member_casts() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(TYPES);
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::AssignTypeMismatch,
        r#"
        ---@param item Signal
        ---@param other Signal
        local function run(item, other)
            ---@cast item.name -?
            item = other
            ---@type string
            local name = item.name
        end
    "#
    ));
}

#[test]
fn member_cast_does_not_remove_nil_from_the_receiver() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    ws.def(TYPES);
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::NeedCheckNil,
        r#"
        local item = parse()
        ---@cast item.name -?
        local name = item.name
    "#
    ));
}
