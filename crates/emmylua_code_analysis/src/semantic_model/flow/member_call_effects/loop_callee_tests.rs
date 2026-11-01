use super::*;

fn check(body: &str, conflicts: bool) {
    let mut ws = crate::VirtualWorkspace::new_with_init_std_lib();
    let mut config = ws.get_emmyrc();
    config.runtime.nonstandard_symbol = serde_json::from_str(r#"["continue"]"#).unwrap();
    ws.update_emmyrc(config);
    let file = ws.def(&format!(
        r#"
local receiver = {{value = 1}}
local other = {{value = 1}}
local function write(target) target.value = nil end
local harmless = function() end
local action = harmless
{body}
"#
    ));
    let model = ws.analysis.semantic_model(file);
    let chunk = model.chunk().unwrap();
    let index = chunk
        .descendants::<LuaIndexExpr>()
        .find(|index| index.get_text() == "receiver.value")
        .unwrap();
    let call = chunk
        .descendants::<LuaCallExpr>()
        .find(|call| {
            call.get_prefix_expr()
                .is_some_and(|prefix| prefix.get_text() == "action")
        })
        .unwrap();
    assert_eq!(
        calls_conflict(&model, &index, call.syntax(), call.get_range().end()),
        conflicts,
        "{body}"
    );
}

#[test]
fn loop_carried_name_callee_keeps_visible_writes() {
    for (start, end) in [
        ("while flag do", "end"),
        ("while true do", "end"),
        ("repeat", "until flag"),
        ("for i = 1, 2 do", "end"),
        ("for i = 2, 1, -1 do", "end"),
        ("for _, item in pairs(items) do", "end"),
    ] {
        check(
            &format!(
                "{start} action(receiver) local observed = receiver.value action = write {end}"
            ),
            true,
        );
    }
}

#[test]
fn loop_callee_replacements_stop_old_values() {
    for body in [
        "while flag do action = harmless action(receiver) local observed = receiver.value action = write end",
        "while flag do action(receiver) local observed = receiver.value action = write action = harmless end",
        "while flag do if choice then action = harmless else action = harmless end action(receiver) local observed = receiver.value action = write end",
        "while flag do local action = harmless action(receiver) local observed = receiver.value action = write end",
    ] {
        check(body, false);
    }
}

#[test]
fn loop_callees_preserve_receiver_and_closure_identity() {
    for body in [
        "while flag do action(other) local observed = receiver.value action = write end",
        "while flag do action(receiver) local observed = receiver.value local function unused() action = write end end",
        "action(receiver) local observed = receiver.value while flag do action = write end",
        "action(receiver) local observed = receiver.value action = write",
        "for i = action(receiver), 10 do local observed = receiver.value action = write end",
        "for item in action(receiver) do local observed = receiver.value action = write end",
        "while flag do local function nested() action(receiver) local observed = receiver.value end action = write end",
    ] {
        check(body, false);
    }
}

#[test]
fn terminating_loop_paths_do_not_feed_the_next_iteration() {
    for body in [
        "while flag do action(receiver) local observed = receiver.value action = write break end",
        "while flag do action(receiver) local observed = receiver.value action = write return end",
        "while flag do action(receiver) local observed = receiver.value if choice then action = write break end end",
        "repeat action(receiver) local observed = receiver.value action = write until true",
        "for i = 1, 1 do action(receiver) local observed = receiver.value action = write end",
        "for i = 1, 1, -1 do action(receiver) local observed = receiver.value action = write end",
        "for i = 1e308, 1e308, 1e308 do action(receiver) local observed = receiver.value action = write end",
        "for i = -1e308, -1e308, -1e308 do action(receiver) local observed = receiver.value action = write end",
    ] {
        check(body, false);
    }
}

#[test]
fn conditional_and_nested_loop_callees_keep_possible_writes() {
    for body in [
        "for i = 1e308, 1e308 do action(receiver) local observed = receiver.value action = write end",
        "while flag do if choice then action = harmless end action(receiver) local observed = receiver.value action = write end",
        "while flag do action(receiver) local observed = receiver.value if choice then action = write end end",
        "while flag do while other_flag do action(receiver) local observed = receiver.value end action = write end",
        "while flag do action(receiver) local observed = receiver.value if choice then action = write else action = harmless end end",
        "while flag do action(receiver) local observed = receiver.value action = write continue end",
        "while flag do action(receiver) local observed = receiver.value if choice then action = write continue end action = harmless end",
    ] {
        check(body, true);
    }
}

#[test]
fn loop_carried_rawset_alias_keeps_visible_writes() {
    check(
        "while flag do action(receiver, 'value', nil) local observed = receiver.value action = rawset end",
        true,
    );
}
