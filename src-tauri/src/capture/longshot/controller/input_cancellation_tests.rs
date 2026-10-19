use super::*;
use std::cell::Cell;

#[test]
fn longshot_input_cancel_controller_retires_cloned_permission() {
    let controller = LongshotController::new();
    let (started, _, gate) = begin_direct(&controller, panorama(64, 72, 33));
    let target = controller.owner_lease(&started.token).unwrap().auto_target;
    let called = Cell::new(0);
    target
        .run_input(|| {
            called.set(called.get() + 1);
            Ok(())
        })
        .unwrap();
    cancel_and_release(&controller, &started.token);
    let result = target.run_input(|| {
        called.set(called.get() + 1);
        Ok(())
    });
    assert_eq!(result.unwrap_err().code(), "longshot_session_superseded");
    assert_eq!(called.get(), 1);
    assert_eq!(gate.active_mode().unwrap(), None);
}

#[test]
fn longshot_input_cancel_old_token_cannot_retire_next_owner_permission() {
    let controller = LongshotController::with_test_state(repeated_id, 0);
    let (old, _, gate) = begin_direct(&controller, panorama(64, 72, 34));
    let old_target = controller.owner_lease(&old.token).unwrap().auto_target;
    cancel_and_release(&controller, &old.token);
    let (capture, selection, _) =
        ordinary_capture(panorama(64, 72, 35), Arc::clone(&gate), vec![], vec![]);
    let next = controller.begin(&capture, &selection).unwrap();
    let next_target = controller
        .owner_lease(&next.start.token)
        .unwrap()
        .auto_target;
    assert_eq!(
        controller.cancel(&old.token).unwrap_err().code(),
        "longshot_session_superseded"
    );
    let old_called = Cell::new(false);
    let result = old_target.run_input(|| {
        old_called.set(true);
        Ok(())
    });
    let next_called = Cell::new(false);
    next_target
        .run_input(|| {
            next_called.set(true);
            Ok(())
        })
        .unwrap();
    cancel_and_release(&controller, &next.start.token);
    assert_eq!(result.unwrap_err().code(), "longshot_session_superseded");
    assert!(!old_called.get());
    assert!(next_called.get());
    assert_eq!(gate.active_mode().unwrap(), None);
}
