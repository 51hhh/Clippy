/// 成功关闭的资源不再调用 Close，失败资源仍可被 Drop 或显式调用重试。
#[derive(Debug, Default)]
pub(super) struct RuntimeCloseState {
    session_closed: bool,
    frame_pool_closed: bool,
}

impl RuntimeCloseState {
    pub fn close<E>(
        &mut self,
        close_session: impl FnOnce() -> Result<(), E>,
        close_frame_pool: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        // 保留 session 错误也要尝试 pool；只记成功，Drop 才能重试失败资源。
        let session = close_pending(&mut self.session_closed, close_session);
        let frame_pool = close_pending(&mut self.frame_pool_closed, close_frame_pool);
        session.and(frame_pool)
    }
}

fn close_pending<E>(closed: &mut bool, close: impl FnOnce() -> Result<(), E>) -> Result<(), E> {
    if *closed {
        return Ok(());
    }
    let result = close();
    *closed = result.is_ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    fn successful_close_is_ordered_and_idempotent() {
        let mut state = RuntimeCloseState::default();
        let events = RefCell::new(Vec::new());
        state
            .close::<&str>(
                || {
                    events.borrow_mut().push("session");
                    Ok(())
                },
                || {
                    events.borrow_mut().push("pool");
                    Ok(())
                },
            )
            .unwrap();
        state
            .close::<&str>(
                || panic!("closed session retried"),
                || panic!("closed pool retried"),
            )
            .unwrap();
        assert_eq!(*events.borrow(), ["session", "pool"]);
    }

    #[test]
    fn session_error_still_closes_pool_and_only_retries_session() {
        let mut state = RuntimeCloseState::default();
        let pool_calls = Cell::new(0);
        assert_eq!(
            state.close(
                || Err("session"),
                || {
                    pool_calls.set(pool_calls.get() + 1);
                    Ok(())
                },
            ),
            Err("session")
        );
        assert_eq!(pool_calls.get(), 1);
        assert!(!state.session_closed);
        assert!(state.frame_pool_closed);
        let session_calls = Cell::new(0);
        state
            .close::<&str>(
                || {
                    session_calls.set(session_calls.get() + 1);
                    Ok(())
                },
                || panic!("successful pool close repeated"),
            )
            .unwrap();
        assert_eq!(session_calls.get(), 1);
        assert!(state.session_closed);
    }

    #[test]
    fn pool_error_only_retries_pool_after_session_succeeded() {
        let mut state = RuntimeCloseState::default();
        assert_eq!(state.close(|| Ok(()), || Err("pool")), Err("pool"));
        assert!(state.session_closed);
        assert!(!state.frame_pool_closed);
        let pool_calls = Cell::new(0);
        state
            .close::<&str>(
                || panic!("successful session close repeated"),
                || {
                    pool_calls.set(pool_calls.get() + 1);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(pool_calls.get(), 1);
    }

    #[test]
    fn both_errors_keep_first_error_and_retry_both() {
        let mut state = RuntimeCloseState::default();
        let events = RefCell::new(Vec::new());
        assert_eq!(
            state.close(
                || {
                    events.borrow_mut().push("session-error");
                    Err("session")
                },
                || {
                    events.borrow_mut().push("pool-error");
                    Err("pool")
                },
            ),
            Err("session")
        );
        state
            .close::<&str>(
                || {
                    events.borrow_mut().push("session-ok");
                    Ok(())
                },
                || {
                    events.borrow_mut().push("pool-ok");
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            *events.borrow(),
            ["session-error", "pool-error", "session-ok", "pool-ok"]
        );
    }

    #[test]
    fn persistent_failures_never_become_success() {
        let mut state = RuntimeCloseState::default();
        let pool_calls = Cell::new(0);
        for _ in 0..2 {
            assert_eq!(
                state.close(
                    || Err("session"),
                    || {
                        pool_calls.set(pool_calls.get() + 1);
                        Err("pool")
                    },
                ),
                Err("session")
            );
        }
        assert_eq!(pool_calls.get(), 2);
        assert!(!state.session_closed);
        assert!(!state.frame_pool_closed);
    }

    #[test]
    fn drop_after_explicit_error_retries_only_failed_resource() {
        struct Owner {
            state: RuntimeCloseState,
            calls: Rc<Cell<(usize, usize)>>,
        }
        impl Owner {
            fn close(&mut self) -> Result<(), &'static str> {
                self.state.close(
                    || {
                        let (session, pool) = self.calls.get();
                        self.calls.set((session + 1, pool));
                        if session == 0 { Err("session") } else { Ok(()) }
                    },
                    || {
                        let (session, pool) = self.calls.get();
                        self.calls.set((session, pool + 1));
                        Ok(())
                    },
                )
            }
        }
        impl Drop for Owner {
            fn drop(&mut self) {
                let _ = self.close();
            }
        }
        let calls = Rc::new(Cell::new((0, 0)));
        let mut owner = Owner {
            state: RuntimeCloseState::default(),
            calls: Rc::clone(&calls),
        };
        assert_eq!(owner.close(), Err("session"));
        drop(owner);
        assert_eq!(calls.get(), (2, 1));
    }
}
