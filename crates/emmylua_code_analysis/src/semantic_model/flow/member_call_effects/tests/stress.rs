//! Deep and wide call graphs must retain visible writes without truncation.

use super::assert_known_call_conflicts;

#[test]
fn deep_known_callees_do_not_hide_conflicting_writes() {
    let mut definitions = "local methods = {}; function methods.unrelated() end\nlocal function step0(target) target.value = nil end\n".to_string();
    for step in 1..2050 {
        definitions.push_str(&format!(
            "local function step{step}(target) step{}(target) end\n",
            step - 1
        ));
    }
    assert_known_call_conflicts(&definitions, "step2049(receiver)");
}

#[test]
fn wide_known_callees_do_not_hide_conflicting_writes() {
    let mut definitions = "local function write(target) target.value = nil end\n".to_string();
    for step in 0..2048 {
        definitions.push_str(&format!(
            "local function harmless{step}(target) return target end\n"
        ));
    }
    definitions.push_str("local function outer(target)\nwrite(target)\n");
    for step in 0..2048 {
        definitions.push_str(&format!("harmless{step}(target)\n"));
    }
    definitions.push_str("end");
    assert_known_call_conflicts(&definitions, "outer(receiver)");
}
