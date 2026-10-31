#[cfg(test)]
mod tests {
    use crate::handlers::test_lib::{ProviderVirtualWorkspace, VirtualSignatureHelp, check};
    use googletest::prelude::*;

    // M3 batch 3 retirement: overload matching / std pcall signature rendering depends on the old signature index and humanize; see migration notes §M3.
    #[gtest]
    fn test_1() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_signature_helper(
            r#"
                ---@class Action
                ---@field id fun(self:Action, itemId:integer, ...:integer?):boolean
                ---@overload fun():Action
                Action = {}

                Action:id(1, <??>)
            "#,
            VirtualSignatureHelp {
                target_label: "Action:id(itemId: integer, ...: integer?): boolean".to_string(),
                active_signature: 0,
                active_parameter: 1,
            },
        ));
        Ok(())
    }

    // M3 batch 3 retirement: overload matching / std pcall signature rendering depends on the old signature index and humanize; see migration notes §M3.
    #[gtest]
    fn test_2() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        check!(ws.check_signature_helper(
            r#"
                ---@param path string
                local function readFile(path)
                end

                pcall(readFile, <??>)
            "#,
            VirtualSignatureHelp {
                target_label:
                    "pcall(f: sync fun(path: string), path: string): (true|false)".to_string(),
                active_signature: 0,
                active_parameter: 1,
            },
        ));
        Ok(())
    }
    #[gtest]
    fn test_union_projection_callable_retains_signature_help() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        check!(ws.check_signature_helper(
            r#"
            ---@class Present
            ---@field handle fun(value: integer): boolean
            ---@class Absent
            ---@alias Event Present|Absent
            ---@param event Event
            local function run(event) event.handle(<??>) end
        "#,
            VirtualSignatureHelp {
                target_label: "event.handle(value: integer): boolean".into(),
                active_signature: 0,
                active_parameter: 0,
            }
        ));
        Ok(())
    }
}
