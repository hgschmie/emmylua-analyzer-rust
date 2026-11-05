use crate::{DiagnosticCode, VirtualWorkspace};

#[test]
fn caller_generic_bounds_survive_undocumented_functions() {
    let mut ws = VirtualWorkspace::new();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        local function unrelated() end
        ---@class BoundingBox
        ---@field width number
        local Box = {}
        local Area = {}
        ---@param area BoundingBox
        ---@return number
        function Area.size(area) return area.width end
        ---@generic T: BoundingBox
        ---@generic U: BoundingBox
        ---@param area T
        ---@param area2 U
        ---@return T|U
        function Area.min(area, area2)
            return (Area.size(area) <= Area.size(area2) and area) or area2
        end
        return Area, Box, unrelated
        "#,
    ));
}

#[test]
fn caller_generic_bounds_use_the_owning_function() {
    // The unused earlier function is a decoy for the legacy file-wide lookup.
    // Only caller's U may determine whether consume(value) is valid.
    for (other_bound, bound, accepted) in [("string", "number", true), ("number", "string", false)]
    {
        let source = format!(
            r#"
            ---@generic U: {other_bound}
            ---@param value U
            local function other(value) return value end
            ---@param value number
            local function consume(value) return value end
            ---@generic U: {bound}
            ---@param value U
            local function caller(value) return consume(value) end
            return other, caller
            "#
        );
        let mut ws = VirtualWorkspace::new();
        assert_eq!(
            ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, &source),
            accepted,
            "{source}"
        );
    }
}
