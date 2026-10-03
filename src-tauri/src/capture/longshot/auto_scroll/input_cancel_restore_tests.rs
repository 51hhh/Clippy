use super::*;
use std::cell::RefCell;
use std::rc::Rc;

struct PointerFixture {
    moves: Rc<RefCell<Vec<(i32, i32)>>>,
}

impl PointerControl for PointerFixture {
    fn location(&self) -> Result<(i32, i32), CaptureError> {
        Ok((100, 100))
    }

    fn move_to(&self, point: (i32, i32)) -> Result<(), CaptureError> {
        self.moves.borrow_mut().push(point);
        Ok(())
    }
}

#[test]
fn longshot_input_cancel_revokes_late_cursor_restore() {
    let target = LongshotAutoTarget::new((100, 100));
    let moves = Rc::new(RefCell::new(Vec::new()));
    let guard = CursorRestore {
        original: (20, 30),
        automatic_point: target.point,
        armed: true,
        pointer: PermittedPointer {
            permission: target.input_permission.clone(),
            pointer: PointerFixture {
                moves: Rc::clone(&moves),
            },
        },
    };
    target.revoke_input();
    target.wait_input_idle().unwrap();
    drop(guard);
    assert!(moves.borrow().is_empty(), "迟到的恢复不得移动指针");
}
