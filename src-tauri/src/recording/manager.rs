//! 单活动录屏注册表。
//!
//! Starting、Recording 与 Stopping 都占用唯一槽位；所有控制命令必须携带精确代次 token。这样一次
//! 录屏的迟到暂停、停止或取消不能操作随后创建的新会话，构建失败也会由 reservation 的 Drop 清槽。

use super::session::{
    DiagnosticRecordingConfig, DiagnosticRecordingError, DiagnosticRecordingReport,
    DiagnosticRecordingSession,
};
use super::worker::RecordingFrameSource;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingToken {
    pub session_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RecordingManagerStatus {
    Idle,
    Starting(RecordingToken),
    Recording(RecordingToken),
    Stopping(RecordingToken),
}

#[derive(Debug, Error)]
pub(super) enum RecordingManagerError {
    #[error("已有录屏会话正在启动、录制或停止")]
    Busy,
    #[error("录屏会话代次已经耗尽")]
    GenerationExhausted,
    #[error("录屏会话 token 已失效")]
    StaleToken,
    #[error("录屏注册表锁已损坏")]
    Poisoned,
    #[error(transparent)]
    Session(#[from] DiagnosticRecordingError),
}

#[derive(Clone)]
pub(super) struct RecordingManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    state: Mutex<ManagerState>,
}

struct ManagerState {
    last_generation: u64,
    slot: ManagerSlot,
}

enum ManagerSlot {
    Idle,
    Starting {
        token: RecordingToken,
        identity: Arc<()>,
    },
    Recording {
        token: RecordingToken,
        identity: Arc<()>,
        session: Box<DiagnosticRecordingSession>,
    },
    Stopping {
        token: RecordingToken,
        identity: Arc<()>,
    },
}

struct StartReservation {
    manager: Weak<ManagerInner>,
    token: RecordingToken,
    identity: Arc<()>,
    committed: bool,
}

impl Default for RecordingManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ManagerInner {
                state: Mutex::new(ManagerState {
                    last_generation: 0,
                    slot: ManagerSlot::Idle,
                }),
            }),
        }
    }

    pub fn start<S>(
        &self,
        app_data_dir: &Path,
        config: DiagnosticRecordingConfig,
        source: S,
    ) -> Result<RecordingToken, RecordingManagerError>
    where
        S: RecordingFrameSource,
    {
        let reservation = self.reserve(config.session_id.clone())?;
        let session = DiagnosticRecordingSession::start(app_data_dir, config, source)?;
        self.commit_start(reservation, session)
    }

    pub fn pause(&self, token: &RecordingToken) -> Result<(), RecordingManagerError> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| RecordingManagerError::Poisoned)?;
        let ManagerSlot::Recording {
            token: active,
            session,
            ..
        } = &state.slot
        else {
            return Err(RecordingManagerError::StaleToken);
        };
        ensure_token(active, token)?;
        session.pause()?;
        Ok(())
    }

    pub fn resume(&self, token: &RecordingToken) -> Result<(), RecordingManagerError> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| RecordingManagerError::Poisoned)?;
        let ManagerSlot::Recording {
            token: active,
            session,
            ..
        } = &state.slot
        else {
            return Err(RecordingManagerError::StaleToken);
        };
        ensure_token(active, token)?;
        session.resume()?;
        Ok(())
    }

    pub fn stop(
        &self,
        token: &RecordingToken,
    ) -> Result<DiagnosticRecordingReport, RecordingManagerError> {
        let (session, identity) = self.take_for_stopping(token)?;
        let result = (*session).stop().map_err(RecordingManagerError::from);
        self.settle_stopping(token, &identity)?;
        result
    }

    pub fn cancel(&self, token: &RecordingToken) -> Result<(), RecordingManagerError> {
        let (session, identity) = self.take_for_stopping(token)?;
        drop(session);
        self.settle_stopping(token, &identity)
    }

    pub fn status(&self) -> Result<RecordingManagerStatus, RecordingManagerError> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| RecordingManagerError::Poisoned)?;
        Ok(match &state.slot {
            ManagerSlot::Idle => RecordingManagerStatus::Idle,
            ManagerSlot::Starting { token, .. } => RecordingManagerStatus::Starting(token.clone()),
            ManagerSlot::Recording { token, .. } => {
                RecordingManagerStatus::Recording(token.clone())
            }
            ManagerSlot::Stopping { token, .. } => RecordingManagerStatus::Stopping(token.clone()),
        })
    }

    fn reserve(&self, session_id: String) -> Result<StartReservation, RecordingManagerError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| RecordingManagerError::Poisoned)?;
        if !matches!(state.slot, ManagerSlot::Idle) {
            return Err(RecordingManagerError::Busy);
        }
        let generation = state
            .last_generation
            .checked_add(1)
            .ok_or(RecordingManagerError::GenerationExhausted)?;
        state.last_generation = generation;
        let token = RecordingToken {
            session_id,
            generation,
        };
        let identity = Arc::new(());
        state.slot = ManagerSlot::Starting {
            token: token.clone(),
            identity: Arc::clone(&identity),
        };
        Ok(StartReservation {
            manager: Arc::downgrade(&self.inner),
            token,
            identity,
            committed: false,
        })
    }

    fn commit_start(
        &self,
        mut reservation: StartReservation,
        session: DiagnosticRecordingSession,
    ) -> Result<RecordingToken, RecordingManagerError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| RecordingManagerError::Poisoned)?;
        let ManagerSlot::Starting { token, identity } = &state.slot else {
            drop(session);
            return Err(RecordingManagerError::StaleToken);
        };
        ensure_token(token, &reservation.token)?;
        if !Arc::ptr_eq(identity, &reservation.identity) {
            drop(session);
            return Err(RecordingManagerError::StaleToken);
        }
        state.slot = ManagerSlot::Recording {
            token: reservation.token.clone(),
            identity: Arc::clone(&reservation.identity),
            session: Box::new(session),
        };
        reservation.committed = true;
        Ok(reservation.token.clone())
    }

    fn take_for_stopping(
        &self,
        token: &RecordingToken,
    ) -> Result<(Box<DiagnosticRecordingSession>, Arc<()>), RecordingManagerError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| RecordingManagerError::Poisoned)?;
        let slot = std::mem::replace(&mut state.slot, ManagerSlot::Idle);
        match slot {
            ManagerSlot::Recording {
                token: active,
                identity,
                session,
            } => {
                if let Err(error) = ensure_token(&active, token) {
                    state.slot = ManagerSlot::Recording {
                        token: active,
                        identity,
                        session,
                    };
                    return Err(error);
                }
                state.slot = ManagerSlot::Stopping {
                    token: active,
                    identity: Arc::clone(&identity),
                };
                Ok((session, identity))
            }
            other => {
                state.slot = other;
                Err(RecordingManagerError::StaleToken)
            }
        }
    }

    fn settle_stopping(
        &self,
        token: &RecordingToken,
        identity: &Arc<()>,
    ) -> Result<(), RecordingManagerError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| RecordingManagerError::Poisoned)?;
        let ManagerSlot::Stopping {
            token: active,
            identity: active_identity,
        } = &state.slot
        else {
            return Err(RecordingManagerError::StaleToken);
        };
        ensure_token(active, token)?;
        if !Arc::ptr_eq(active_identity, identity) {
            return Err(RecordingManagerError::StaleToken);
        }
        state.slot = ManagerSlot::Idle;
        Ok(())
    }
}

impl Drop for StartReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let Ok(mut state) = manager.state.lock() else {
            return;
        };
        let ManagerSlot::Starting { token, identity } = &state.slot else {
            return;
        };
        if token == &self.token && Arc::ptr_eq(identity, &self.identity) {
            state.slot = ManagerSlot::Idle;
        }
    }
}

fn ensure_token(
    active: &RecordingToken,
    provided: &RecordingToken,
) -> Result<(), RecordingManagerError> {
    if active == provided {
        Ok(())
    } else {
        Err(RecordingManagerError::StaleToken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::frame::CapturedFrame;
    use crate::recording::session::RecordingEncoder;
    use std::convert::Infallible;

    struct FixtureSource {
        sequence: u64,
        timestamp_ns: u64,
    }

    impl RecordingFrameSource for FixtureSource {
        type Error = Infallible;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            let sequence = self.sequence;
            let captured_at_ns = self.timestamp_ns;
            self.sequence += 1;
            self.timestamp_ns += 100_000_000;
            Ok(CapturedFrame {
                sequence,
                captured_at_ns,
                width: 2,
                height: 2,
                stride: 8,
                rgba: vec![sequence as u8; 16].into_boxed_slice(),
            })
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            let timestamp_ns = self.timestamp_ns;
            self.timestamp_ns += 1;
            Ok(timestamp_ns)
        }
    }

    fn source() -> FixtureSource {
        FixtureSource {
            sequence: 0,
            timestamp_ns: 100,
        }
    }

    fn config(session_id: &str) -> DiagnosticRecordingConfig {
        DiagnosticRecordingConfig {
            session_id: session_id.to_string(),
            source_id: "fixture".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 2,
            height: 2,
            frames_per_second: 10,
            include_cursor: true,
            encoder: RecordingEncoder::MjpegDiagnostic { jpeg_quality: 85 },
        }
    }

    #[test]
    fn exact_token_controls_and_stops_one_active_session() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new();
        let token = manager
            .start(temporary.path(), config("first"), source())
            .unwrap();
        assert_eq!(
            manager.status().unwrap(),
            RecordingManagerStatus::Recording(token.clone())
        );
        manager.pause(&token).unwrap();
        manager.resume(&token).unwrap();
        let report = manager.stop(&token).unwrap();
        assert!(report.segment_path.exists());
        assert_eq!(manager.status().unwrap(), RecordingManagerStatus::Idle);
    }

    #[test]
    fn busy_start_does_not_create_a_second_session() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new();
        let first = manager
            .start(temporary.path(), config("first"), source())
            .unwrap();
        assert!(matches!(
            manager.start(temporary.path(), config("second"), source()),
            Err(RecordingManagerError::Busy)
        ));
        assert!(!temporary.path().join("recordings/second").exists());
        manager.cancel(&first).unwrap();
    }

    #[test]
    fn stale_token_cannot_control_or_stop_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new();
        let first = manager
            .start(temporary.path(), config("first"), source())
            .unwrap();
        manager.cancel(&first).unwrap();
        let second = manager
            .start(temporary.path(), config("second"), source())
            .unwrap();
        assert!(matches!(
            manager.pause(&first),
            Err(RecordingManagerError::StaleToken)
        ));
        assert!(matches!(
            manager.stop(&first),
            Err(RecordingManagerError::StaleToken)
        ));
        assert_eq!(
            manager.status().unwrap(),
            RecordingManagerStatus::Recording(second.clone())
        );
        manager.cancel(&second).unwrap();
    }

    #[test]
    fn abandoned_start_reservation_clears_only_its_generation() {
        let manager = RecordingManager::new();
        let reservation = manager.reserve("first".to_string()).unwrap();
        let first = reservation.token.clone();
        assert_eq!(
            manager.status().unwrap(),
            RecordingManagerStatus::Starting(first)
        );
        drop(reservation);
        assert_eq!(manager.status().unwrap(), RecordingManagerStatus::Idle);
        let next = manager.reserve("second".to_string()).unwrap();
        assert_eq!(next.token.generation, 2);
    }
}
