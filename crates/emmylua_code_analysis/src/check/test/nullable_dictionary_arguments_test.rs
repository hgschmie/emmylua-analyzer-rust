use std::sync::Arc;

use crate::{DiagnosticCode, VirtualWorkspace};
use tokio_util::sync::CancellationToken;

#[test]
fn nullable_class_dictionary_argument_requires_its_own_guard() {
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    let mut config = (*ws.analysis.get_emmyrc()).clone();
    config.strict.table_index = true;
    ws.analysis.update_config(Arc::new(config));
    ws.analysis
        .diagnostic
        .enable_only(DiagnosticCode::ParamTypeMismatch);
    let file = ws.def(
        r#"
---@class WireConnector
---@field connect_to fun(target: WireConnector, reach_check?: boolean)
---@param input table<integer, WireConnector>
---@param lamp table<integer, WireConnector>
local function connect(input, lamp)
    assert(input[1])
    input[1].connect_to(lamp[1], false)
    local target = lamp[1]
    if target then input[1].connect_to(target, false) end
    input[1].connect_to(assert(target), false)
    assert(lamp[1])
    input[1].connect_to(lamp[1], false)
end
"#,
    );
    let diagnostics = ws
        .analysis
        .diagnose_file(file, CancellationToken::new())
        .unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].range.start.line, 7);
    assert!(
        diagnostics[0].message.contains("WireConnector?"),
        "{diagnostics:#?}"
    );
}
