use scopeguard::{ScopeGuard, guard};

/// pool 初始化期间保留回滚所有权，成功才交给完整 runtime。
pub(super) fn initialize_pool<P, S, E>(
    pool: P,
    register: impl FnOnce(&P) -> Result<(), E>,
    create_session: impl FnOnce(&P) -> Result<S, E>,
    close_pool: impl FnOnce(P),
) -> Result<(P, S), E> {
    let pool = guard(pool, close_pool);
    register(&pool)?;
    let session = create_session(&pool)?;
    Ok((ScopeGuard::into_inner(pool), session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Events = Rc<RefCell<Vec<&'static str>>>;

    #[derive(Debug)]
    struct Pool {
        events: Events,
        close_fails: bool,
    }

    impl Pool {
        fn close(&self) -> Result<(), &'static str> {
            self.events.borrow_mut().push("close");
            if self.close_fails {
                Err("close")
            } else {
                Ok(())
            }
        }
    }

    impl Drop for Pool {
        fn drop(&mut self) {
            self.events.borrow_mut().push("pool-drop");
        }
    }

    fn rollback(pool: Pool) {
        if pool.close().is_err() {
            pool.events.borrow_mut().push("close-error");
        }
    }

    fn pool(close_fails: bool) -> (Pool, Events) {
        let events = Events::default();
        (
            Pool {
                events: Rc::clone(&events),
                close_fails,
            },
            events,
        )
    }

    #[test]
    fn registration_failure_closes_before_drop_without_creating_session() {
        let (pool, events) = pool(false);
        let result = initialize_pool::<_, (), _>(
            pool,
            |pool| {
                pool.events.borrow_mut().push("register");
                Err("register")
            },
            |_| panic!("session created after registration failure"),
            rollback,
        );
        assert_eq!(result.unwrap_err(), "register");
        assert_eq!(*events.borrow(), ["register", "close", "pool-drop"]);
    }

    #[test]
    fn session_failure_closes_registered_pool_before_drop() {
        let (pool, events) = pool(false);
        let result = initialize_pool::<_, (), _>(
            pool,
            |pool| {
                pool.events.borrow_mut().push("register");
                Ok(())
            },
            |pool| {
                pool.events.borrow_mut().push("session");
                Err("session")
            },
            rollback,
        );
        assert_eq!(result.unwrap_err(), "session");
        assert_eq!(
            *events.borrow(),
            ["register", "session", "close", "pool-drop"]
        );
    }

    #[test]
    fn rollback_close_failure_preserves_original_initialization_error() {
        let (pool, events) = pool(true);
        let result = initialize_pool::<_, (), _>(
            pool,
            |_| Ok(()),
            |pool| {
                pool.events.borrow_mut().push("session");
                Err("session")
            },
            rollback,
        );
        assert_eq!(result.unwrap_err(), "session");
        assert_eq!(
            *events.borrow(),
            ["session", "close", "close-error", "pool-drop"]
        );
    }

    #[test]
    fn success_transfers_pool_without_early_or_duplicate_close() {
        let (pool, events) = pool(false);
        let (owned_pool, session) = initialize_pool(
            pool,
            |pool| {
                pool.events.borrow_mut().push("register");
                Ok::<_, &str>(())
            },
            |pool| {
                pool.events.borrow_mut().push("session");
                Ok("owned-session")
            },
            rollback,
        )
        .unwrap();
        assert_eq!(session, "owned-session");
        assert_eq!(*events.borrow(), ["register", "session"]);
        rollback(owned_pool);
        assert_eq!(
            *events.borrow(),
            ["register", "session", "close", "pool-drop"]
        );
    }
}
