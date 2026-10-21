use crate::{DiagnosticCode, VirtualWorkspace};

fn check(code: DiagnosticCode, source: &str, expected: bool) {
    let mut ws = VirtualWorkspace::new();
    assert_eq!(ws.has_no_diagnostic(code, source), expected, "{source}");
}

#[test]
fn array_cast_matches_named_union_alternative() {
    check(
        DiagnosticCode::CastTypeMismatch,
        r#"
---@alias Text string|string[]
---@alias Message Text
---@param parts string[]
local function message(parts)
    ---@cast parts Message
    return parts
end
"#,
        true,
    );
}

#[test]
fn array_return_matches_named_union_alternative() {
    check(
        DiagnosticCode::ReturnTypeMismatch,
        r#"
---@alias Text string|string[]
---@param parts string[]
---@return Text
local function message(parts) return parts end
"#,
        true,
    );
}

#[test]
fn array_argument_matches_named_union_alternative() {
    check(
        DiagnosticCode::ParamTypeMismatch,
        r#"
---@alias Text string|string[]
---@param message Text
local function accept(message) end
---@param parts string[]
local function message(parts) accept(parts) end
"#,
        true,
    );
}

#[test]
fn recursive_array_alias_preserves_cast_return_and_argument_types() {
    let source = r#"
---@alias Message string|number|boolean|nil|Message[]
---@param message Message
local function accept(message) end
---@param parts Message[]
---@return Message
local function message(parts)
    accept(parts)
    ---@cast parts Message
    return parts
end
---@param parts Message[][]
---@return Message
local function nested(parts) return parts end
"#;
    for code in [
        DiagnosticCode::CastTypeMismatch,
        DiagnosticCode::ReturnTypeMismatch,
        DiagnosticCode::ParamTypeMismatch,
    ] {
        check(code, source, true);
    }
}

#[test]
fn optional_array_union_accepts_array_values() {
    check(
        DiagnosticCode::CastTypeMismatch,
        r#"
---@alias Text string|string[]
---@param parts string[]
local function message(parts)
    ---@cast parts Text?
    return parts
end
"#,
        true,
    );
}

#[test]
fn array_union_rejects_incompatible_element_types() {
    for (code, source) in [
        (
            DiagnosticCode::CastTypeMismatch,
            r#"
---@alias Text string|string[]
---@param parts boolean[]
local function message(parts)
    ---@cast parts Text
    return parts
end
"#,
        ),
        (
            DiagnosticCode::ReturnTypeMismatch,
            r#"
---@alias Text string|string[]
---@param parts boolean[]
---@return Text
local function message(parts) return parts end
"#,
        ),
        (
            DiagnosticCode::ParamTypeMismatch,
            r#"
---@alias Text string|string[]
---@param message Text
local function accept(message) end
---@param parts boolean[]
local function message(parts) accept(parts) end
"#,
        ),
        (
            DiagnosticCode::CastTypeMismatch,
            r#"
---@alias Message string|Message[]
---@param parts boolean[]
local function message(parts)
    ---@cast parts Message
    return parts
end
"#,
        ),
    ] {
        check(code, source, false);
    }
}

#[test]
fn array_union_does_not_admit_nil_or_unrelated_scalars() {
    check(
        DiagnosticCode::ReturnTypeMismatch,
        r#"
---@alias Text string|string[]
---@return Text
local function message() return nil end
"#,
        false,
    );
    check(
        DiagnosticCode::CastTypeMismatch,
        r#"
---@param parts string[]
local function message(parts)
    ---@cast parts number|boolean
    return parts
end
"#,
        false,
    );
}

#[test]
fn array_union_alias_cycles_terminate_without_matching() {
    check(
        DiagnosticCode::CastTypeMismatch,
        r#"
---@alias Cycle string|Cycle
---@param parts boolean[]
local function message(parts)
    ---@cast parts Cycle
    return parts
end
"#,
        false,
    );
}
