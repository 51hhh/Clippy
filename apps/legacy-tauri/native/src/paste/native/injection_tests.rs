use super::*;
use enigo::{InputError, InputResult};
use std::cell::RefCell;
use std::rc::Rc;

const V: Key = Key::Unicode('v');

#[derive(Clone, Copy, Default)]
enum ClickOutcome {
    #[default]
    Complete,
    Partial,
    Blocked,
    PanicPartial,
}

#[derive(Default)]
struct State {
    click: ClickOutcome,
    modifier_press_fails: bool,
    v_release_failures: usize,
    modifier_release_failures: usize,
    calls: Vec<(Key, enigo::Direction)>,
    down: Vec<Key>,
    held: Vec<Key>,
}

struct KeyboardFixture(Rc<RefCell<State>>);

impl Keyboard for KeyboardFixture {
    fn fast_text(&mut self, _: &str) -> InputResult<Option<()>> {
        Err(InputError::InvalidInput("fixture never enters text"))
    }

    fn raw(&mut self, _: u16, _: enigo::Direction) -> InputResult<()> {
        Err(InputError::InvalidInput("fixture never enters raw input"))
    }

    fn key(&mut self, key: Key, direction: enigo::Direction) -> InputResult<()> {
        let panic_partial = {
            let mut state = self.0.borrow_mut();
            state.calls.push((key, direction));
            match (key, direction) {
                (Key::Control, Press) => {
                    if state.modifier_press_fails {
                        return Err(InputError::Simulate("controlled modifier press failure"));
                    }
                    state.down.push(key);
                    // 对应 Enigo：只有成功 Press 才记入 held，Click 不记入。
                    state.held.push(key);
                    false
                }
                (V, Click) => match state.click {
                    ClickOutcome::Complete => false,
                    ClickOutcome::Blocked => {
                        return Err(InputError::Simulate("controlled incomplete click"));
                    }
                    ClickOutcome::Partial => {
                        // 模拟两个 INPUT 只插入 V-down，未插入 V-up；不是实际 SendInput。
                        state.down.push(V);
                        return Err(InputError::Simulate("controlled incomplete click"));
                    }
                    ClickOutcome::PanicPartial => {
                        state.down.push(V);
                        true
                    }
                },
                (V, Release) => {
                    if state.v_release_failures > 0 {
                        state.v_release_failures -= 1;
                        return Err(InputError::Simulate("controlled V release failure"));
                    }
                    state.down.retain(|held| *held != V);
                    state.held.retain(|held| *held != V);
                    false
                }
                (Key::Control, Release) => {
                    if state.modifier_release_failures > 0 {
                        state.modifier_release_failures -= 1;
                        return Err(InputError::Simulate("controlled modifier release failure"));
                    }
                    state.down.retain(|held| *held != Key::Control);
                    state.held.retain(|held| *held != Key::Control);
                    false
                }
                _ => return Err(InputError::InvalidInput("unexpected fixture input")),
            }
        };
        // 先释放 RefMut，展开清理才能再进入受控键盘。
        assert!(!panic_partial, "受控 Click 部分发送后展开");
        Ok(())
    }
}

impl Drop for KeyboardFixture {
    fn drop(&mut self) {
        // 对应锁定 Enigo 的默认 Drop：只重试已记录的成功 Press。
        let held = self.0.borrow().held.clone();
        for key in held {
            let _ = self.key(key, Release);
        }
    }
}

fn state(click: ClickOutcome) -> Rc<RefCell<State>> {
    Rc::new(RefCell::new(State {
        click,
        ..State::default()
    }))
}

fn attempt(state: &Rc<RefCell<State>>) -> Result<(), PasteError> {
    let mut keyboard = KeyboardFixture(Rc::clone(state));
    inject_with_keyboard(&mut keyboard, Key::Control, "Control")
}

fn count(state: &Rc<RefCell<State>>, key: Key, direction: enigo::Direction) -> usize {
    state
        .borrow()
        .calls
        .iter()
        .filter(|call| **call == (key, direction))
        .count()
}

fn click_error(result: Result<(), PasteError>) -> String {
    let error = result.unwrap_err();
    assert_eq!(error.code(), "key_injection");
    match error {
        PasteError::KeyInjection { action, detail } => {
            assert_eq!(action, "按下 V");
            assert!(detail.contains("controlled incomplete click"));
            detail
        }
        other => panic!("错误分类改变: {other}"),
    }
}

#[test]
fn partial_click_is_released_before_modifier_and_not_left_to_enigo_drop() {
    let state = state(ClickOutcome::Partial);
    click_error(attempt(&state));
    assert!(state.borrow().down.is_empty(), "未释放部分发送的 V");
    assert_eq!(
        state.borrow().calls,
        [
            (Key::Control, Press),
            (V, Click),
            (V, Release),
            (Key::Control, Release)
        ]
    );
}

#[test]
fn first_v_release_failure_is_retried_at_scope_exit() {
    let state = state(ClickOutcome::Partial);
    state.borrow_mut().v_release_failures = 1;
    let detail = click_error(attempt(&state));
    assert!(state.borrow().down.is_empty(), "V 清理失败后未重试");
    assert_eq!(count(&state, V, Release), 2);
    assert_eq!(count(&state, Key::Control, Release), 1);
    assert!(detail.contains("controlled V release failure"));
}

#[test]
fn v_and_modifier_cleanup_failures_keep_click_error_and_release_both() {
    let state = state(ClickOutcome::Partial);
    {
        let mut state = state.borrow_mut();
        state.v_release_failures = 1;
        state.modifier_release_failures = 1;
    }
    let detail = click_error(attempt(&state));
    assert!(state.borrow().down.is_empty(), "两个已按下键的清理不完整");
    assert_eq!(count(&state, V, Release), 2);
    assert_eq!(count(&state, Key::Control, Release), 2);
    assert!(detail.contains("controlled V release failure"));
    assert!(detail.contains("controlled modifier release failure"));
}

#[test]
fn permanent_v_release_failure_is_bounded_and_never_becomes_success() {
    let state = state(ClickOutcome::Partial);
    state.borrow_mut().v_release_failures = usize::MAX;
    let detail = click_error(attempt(&state));
    assert_eq!(count(&state, V, Release), 2, "清理应尝试两次且不循环");
    assert_eq!(count(&state, Key::Control, Release), 1);
    assert_eq!(state.borrow().down, [V]);
    assert!(detail.contains("controlled V release failure"));
}

#[test]
fn blocked_click_cleans_uncertain_v_and_keeps_primary_error() {
    let state = state(ClickOutcome::Blocked);
    let detail = click_error(attempt(&state));
    assert_eq!(count(&state, V, Release), 1);
    assert!(state.borrow().down.is_empty());
    assert!(!detail.contains("release failure"));
}

#[test]
fn partial_click_unwind_cleans_v_then_recorded_modifier() {
    let state = state(ClickOutcome::PanicPartial);
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| attempt(&state)));
    assert!(unwind.is_err());
    assert!(state.borrow().down.is_empty(), "展开清理遗漏 V");
    assert_eq!(
        state.borrow().calls,
        [
            (Key::Control, Press),
            (V, Click),
            (V, Release),
            (Key::Control, Release)
        ]
    );
}

#[test]
fn successful_click_retains_one_click_and_no_extra_release() {
    let state = state(ClickOutcome::Complete);
    attempt(&state).unwrap();
    assert!(state.borrow().down.is_empty());
    assert_eq!(
        state.borrow().calls,
        [(Key::Control, Press), (V, Click), (Key::Control, Release)]
    );
}

#[test]
fn modifier_press_failure_never_sends_v_or_release() {
    let state = state(ClickOutcome::Complete);
    state.borrow_mut().modifier_press_fails = true;
    let error = attempt(&state).unwrap_err();
    assert!(matches!(error, PasteError::KeyInjection { action, .. } if action == "按下 Control"));
    assert_eq!(state.borrow().calls, [(Key::Control, Press)]);
    assert!(state.borrow().down.is_empty());
}

#[test]
fn modifier_release_failure_keeps_error_and_existing_drop_retry() {
    let state = state(ClickOutcome::Complete);
    state.borrow_mut().modifier_release_failures = 1;
    let error = attempt(&state).unwrap_err();
    assert!(matches!(error, PasteError::KeyInjection { action, .. } if action == "释放 Control"));
    assert!(state.borrow().down.is_empty());
    assert_eq!(count(&state, Key::Control, Release), 2);
    assert_eq!(count(&state, V, Release), 0);
}

#[test]
fn target_validation_failure_never_calls_keyboard() {
    let state = state(ClickOutcome::Complete);
    let result = with_input_backend(
        || Ok(KeyboardFixture(Rc::clone(&state))),
        || Err(PasteError::NativeTargetInvalid),
        |mut keyboard| inject_with_keyboard(&mut keyboard, Key::Control, "Control"),
    );
    assert!(matches!(result, Err(PasteError::NativeTargetInvalid)));
    assert!(state.borrow().calls.is_empty());
}
