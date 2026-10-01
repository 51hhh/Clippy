use super::*;

const FAILURE: &str = "native register failed";

struct RegistrationTrace {
    outcome: Result<(), String>,
    registered: Vec<u32>,
    actions: Vec<(String, String, Result<(), String>)>,
}

fn config(global: &str, pin: &str, capture: &str) -> AppConfig {
    AppConfig {
        global_shortcut: global.into(),
        pin_shortcut: pin.into(),
        capture_shortcut: capture.into(),
        ..AppConfig::default()
    }
}

fn id(raw: &str) -> u32 {
    Shortcut::from_str(raw.trim()).unwrap().id()
}

// 只替换原生注册与状态/事件 adapter；计划、计数与返回值执行生产协议。
fn run(config: &AppConfig, fail_ids: &[u32]) -> RegistrationTrace {
    let mut registered = Vec::new();
    let mut actions = Vec::new();
    let outcome = execute_tauri_registration(
        config,
        |shortcut| {
            registered.push(shortcut.id());
            if fail_ids.contains(&shortcut.id()) {
                Err(FAILURE.into())
            } else {
                Ok(())
            }
        },
        |action, raw, result| actions.push((action.into(), raw.into(), result)),
    );
    RegistrationTrace {
        outcome,
        registered,
        actions,
    }
}

#[test]
fn shared_failure_without_any_registered_key_returns_error_for_both_actions() {
    let trace = run(&config("Alt+V", "Alt+V", ""), &[id("Alt+V")]);
    assert_eq!(trace.registered, [id("Alt+V")]);
    assert_eq!(trace.outcome, Err(FAILURE.into()));
    assert_eq!(
        trace.actions,
        [
            ("global".into(), "Alt+V".into(), Err(FAILURE.into())),
            ("pin".into(), "Alt+V".into(), Err(FAILURE.into())),
            ("capture".into(), "".into(), Ok(())),
        ]
    );
}

#[test]
fn all_three_shared_failures_keep_one_registration_and_three_failures() {
    let trace = run(&config("Alt+V", "Alt+V", "Alt+V"), &[id("Alt+V")]);
    assert_eq!(trace.registered, [id("Alt+V")]);
    assert_eq!(trace.outcome, Err(FAILURE.into()));
    assert!(trace
        .actions
        .iter()
        .all(|(_, _, r)| r == &Err(FAILURE.into())));
}

#[test]
fn independent_key_success_keeps_partial_success_and_shared_failure() {
    let trace = run(&config("Alt+V", "Alt+V", "Ctrl+Shift+A"), &[id("Alt+V")]);
    assert_eq!(trace.registered, [id("Alt+V"), id("Ctrl+Shift+A")]);
    assert_eq!(trace.outcome, Ok(()));
    assert_eq!(trace.actions[0].2, Err(FAILURE.into()));
    assert_eq!(trace.actions[1].2, Err(FAILURE.into()));
    assert_eq!(trace.actions[2].2, Ok(()));
}

#[test]
fn normalized_shared_alias_inherits_failure_and_keeps_trimmed_action_text() {
    assert_eq!(id("Ctrl+Shift+A"), id("Shift+Control+A"));
    let trace = run(
        &config(" Ctrl+Shift+A ", " Shift+Control+A ", ""),
        &[id("Ctrl+Shift+A")],
    );
    assert_eq!(trace.registered, [id("Ctrl+Shift+A")]);
    assert_eq!(trace.outcome, Err(FAILURE.into()));
    assert_eq!(trace.actions[0].1, "Ctrl+Shift+A");
    assert_eq!(trace.actions[1].1, "Shift+Control+A");
    assert_eq!(trace.actions[1].2, Err(FAILURE.into()));
}

#[test]
fn shared_success_registers_once_and_records_success_for_every_action() {
    let config = config("Alt+V", " Alt+V ", "Alt+V");
    let trace = run(&config, &[]);
    assert_eq!(trace.registered, [id("Alt+V")]);
    assert_eq!(trace.outcome, Ok(()));
    assert_eq!(
        trace.actions,
        [
            ("global".into(), "Alt+V".into(), Ok(())),
            ("pin".into(), "Alt+V".into(), Ok(())),
            ("capture".into(), "Alt+V".into(), Ok(())),
        ]
    );
    assert_eq!(
        shortcut_action(&config, &Shortcut::from_str("Alt+V").unwrap()),
        Some(ShortcutAction::ToggleMain)
    );
}

#[test]
fn unset_shortcuts_register_nothing_and_record_success_for_all_actions() {
    let trace = run(&config("", "  ", "\t"), &[]);
    assert!(trace.registered.is_empty());
    assert_eq!(trace.outcome, Ok(()));
    assert_eq!(trace.actions.len(), 3);
    assert!(trace
        .actions
        .iter()
        .all(|(_, raw, r)| raw.is_empty() && r.is_ok()));
}

#[test]
fn invalid_shortcuts_remain_independent_errors_with_no_native_registration() {
    let trace = run(&config("NotAKey+", "NotAKey+", ""), &[]);
    assert!(trace.registered.is_empty());
    let error = trace.outcome.unwrap_err();
    assert!(error.starts_with("快捷键 `NotAKey+` 解析失败"));
    assert_eq!(trace.actions[0].2, Err(error.clone()));
    assert_eq!(trace.actions[1].2, Err(error));
    assert_eq!(trace.actions[2].2, Ok(()));

    let partial = run(&config("NotAKey+", "Alt+V", "Alt+V"), &[]);
    assert_eq!(partial.registered, [id("Alt+V")]);
    assert_eq!(partial.outcome, Ok(()));
    assert!(partial.actions[0].2.is_err());
    assert_eq!(partial.actions[1].2, Ok(()));
    assert_eq!(partial.actions[2].2, Ok(()));
}

#[test]
fn another_successful_shared_group_does_not_inherit_the_first_key_failure() {
    let trace = run(
        &config("Alt+V", "Ctrl+Shift+A", "Control+Shift+A"),
        &[id("Alt+V")],
    );
    assert_eq!(trace.registered, [id("Alt+V"), id("Ctrl+Shift+A")]);
    assert_eq!(trace.outcome, Ok(()));
    assert_eq!(trace.actions[0].2, Err(FAILURE.into()));
    assert_eq!(trace.actions[1].2, Ok(()));
    assert_eq!(trace.actions[2].2, Ok(()));
}

#[test]
fn each_attempt_uses_its_own_outcomes_and_successful_retry_records_success() {
    let config = config("Alt+V", "Alt+V", "");
    let first = run(&config, &[]);
    assert_eq!(first.outcome, Ok(()));
    let failed = run(&config, &[id("Alt+V")]);
    assert_eq!(failed.outcome, Err(FAILURE.into()));
    assert_eq!(failed.actions[1].2, Err(FAILURE.into()));
    let retry = run(&config, &[]);
    assert_eq!(retry.registered, [id("Alt+V")]);
    assert_eq!(retry.outcome, Ok(()));
    assert!(retry.actions.iter().all(|(_, _, r)| r.is_ok()));
}

#[test]
fn shared_total_failure_reaches_the_production_config_rollback_protocol() {
    let mut current = config("Alt+X", "", "");
    let next = config("Alt+V", "Alt+V", "");
    let mut saved = Vec::new();
    let mut applied = Vec::new();
    let outcome = crate::config::commit_config_change(
        &mut current,
        next,
        |value| {
            saved.push(value.global_shortcut.clone());
            Ok(())
        },
        |value| {
            applied.push(value.global_shortcut.clone());
            run(value, &[id("Alt+V")]).outcome
        },
    );
    assert_eq!(outcome, Err(FAILURE.into()));
    assert_eq!(saved, ["Alt+V", "Alt+X"]);
    assert_eq!(applied, ["Alt+V", "Alt+X"]);
    assert_eq!(current.global_shortcut, "Alt+X");
    assert!(current.pin_shortcut.is_empty());
}
