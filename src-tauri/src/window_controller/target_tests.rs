use super::select_main_window_target;
use std::cell::Cell;
use tauri::PhysicalPosition;

type Target = (usize, PhysicalPosition<i32>);

fn saved_target() -> Target {
    (1, PhysicalPosition::new(-1720, 240))
}

#[test]
fn main_window_target_saved_target_survives_failing_fallback() {
    let calls = Cell::new(0);
    let selected = select_main_window_target(Ok(Some(saved_target())), || {
        calls.set(calls.get() + 1);
        Err("native monitor query unavailable".to_string())
    });

    assert_eq!(selected, Ok(Some(saved_target())));
    assert_eq!(calls.get(), 0);
}

#[test]
fn main_window_target_saved_target_skips_successful_fallback() {
    let calls = Cell::new(0);
    let selected = select_main_window_target::<_, String>(Ok(Some(saved_target())), || {
        calls.set(calls.get() + 1);
        Ok(Some((0, PhysicalPosition::new(1900, 700))))
    });

    assert_eq!(selected, Ok(Some(saved_target())));
    assert_eq!(calls.get(), 0);
}

#[test]
fn main_window_target_missing_saved_uses_fallback_once() {
    let calls = Cell::new(0);
    let selected = select_main_window_target::<_, String>(Ok(None), || {
        calls.set(calls.get() + 1);
        Ok(Some(saved_target()))
    });

    assert_eq!(selected, Ok(Some(saved_target())));
    assert_eq!(calls.get(), 1);
}

#[test]
fn main_window_target_no_target_remains_none() {
    let calls = Cell::new(0);
    let selected = select_main_window_target::<Target, String>(Ok(None), || {
        calls.set(calls.get() + 1);
        Ok(None)
    });

    assert_eq!(selected, Ok(None));
    assert_eq!(calls.get(), 1);
}

#[test]
fn main_window_target_fallback_error_is_preserved() {
    let calls = Cell::new(0);
    let selected = select_main_window_target::<Target, _>(Ok(None), || {
        calls.set(calls.get() + 1);
        Err("original fallback error".to_string())
    });

    assert_eq!(selected, Err("original fallback error".to_string()));
    assert_eq!(calls.get(), 1);
}

#[test]
fn main_window_target_remembered_error_does_not_query_fallback() {
    let calls = Cell::new(0);
    let selected = select_main_window_target(Err("original saved-query error".to_string()), || {
        calls.set(calls.get() + 1);
        Ok(Some(saved_target()))
    });

    assert_eq!(selected, Err("original saved-query error".to_string()));
    assert_eq!(calls.get(), 0);
}
