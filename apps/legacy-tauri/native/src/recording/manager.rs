//! 单活动录屏注册表。
//!
//! Starting、Recording 与 Stopping 都占用唯一槽位；所有控制命令必须携带精确代次 token。这样一次
//! 录屏的迟到暂停、停止或取消不能操作随后创建的新会话，构建失败也会由 reservation 的 Drop 清槽。

use super::clock::RecordingSessionClock;
use super::session::{
    DiagnosticRecordingConfig, DiagnosticRecordingError, DiagnosticRecordingReport,
    DiagnosticRecordingSession,
};
use super::worker::RecordingFrameSource;
#[cfg(feature = "recording-opus-webm")]
use super::{
    audio_worker::RecordingAudioSource,
    av_session::{AvRecordingConfig, AvRecordingSession, AvRecordingSessionError},
};
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

/// 控制窗只消费两种 session 共有的停止摘要；双轨的完整音频统计仍保存在 schema v2 manifest。
#[derive(Debug)]
pub(super) struct RecordingSessionReport {
    pub segment_paths: Vec<std::path::PathBuf>,
    pub final_output_path: Option<std::path::PathBuf>,
    pub duration_ns: u64,
    pub captured_frames: u64,
    pub accepted_frames: u64,
    pub encoder_input_frames: u64,
    pub encoded_frames: u64,
    pub dropped_by_backpressure: u64,
    pub audio_packet_count: Option<u64>,
    pub audio_pcm_frame_count: Option<u64>,
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
    #[cfg(feature = "recording-opus-webm")]
    #[error(transparent)]
    AvSession(#[from] AvRecordingSessionError),
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
        session: Box<ManagedRecordingSession>,
    },
    Stopping {
        token: RecordingToken,
        identity: Arc<()>,
    },
}

enum ManagedRecordingSession {
    Video(DiagnosticRecordingSession),
    #[cfg(feature = "recording-opus-webm")]
    AudioVideo(AvRecordingSession),
}

impl ManagedRecordingSession {
    fn pause(&self) -> Result<(), RecordingManagerError> {
        match self {
            Self::Video(session) => session.pause().map_err(Into::into),
            #[cfg(feature = "recording-opus-webm")]
            Self::AudioVideo(session) => session.pause().map_err(Into::into),
        }
    }

    fn resume(&self) -> Result<(), RecordingManagerError> {
        match self {
            Self::Video(session) => session.resume().map_err(Into::into),
            #[cfg(feature = "recording-opus-webm")]
            Self::AudioVideo(session) => session.resume().map_err(Into::into),
        }
    }

    fn stop(self) -> Result<RecordingSessionReport, RecordingManagerError> {
        match self {
            Self::Video(session) => session.stop().map(Into::into).map_err(Into::into),
            #[cfg(feature = "recording-opus-webm")]
            Self::AudioVideo(session) => session.stop().map(Into::into).map_err(Into::into),
        }
    }

    fn has_terminated_worker(&self) -> bool {
        match self {
            Self::Video(session) => session.has_terminated_worker(),
            #[cfg(feature = "recording-opus-webm")]
            Self::AudioVideo(session) => session.has_terminated_worker(),
        }
    }
}

impl From<DiagnosticRecordingReport> for RecordingSessionReport {
    fn from(report: DiagnosticRecordingReport) -> Self {
        Self {
            segment_paths: report.segment_paths,
            final_output_path: report.final_output_path,
            duration_ns: report.duration_ns,
            captured_frames: report.captured_frames,
            accepted_frames: report.accepted_frames,
            encoder_input_frames: report.encoder_input_frames,
            encoded_frames: report.encoded_frames,
            dropped_by_backpressure: report.dropped_by_backpressure,
            audio_packet_count: None,
            audio_pcm_frame_count: None,
        }
    }
}

#[cfg(feature = "recording-opus-webm")]
impl From<super::av_session::AvRecordingReport> for RecordingSessionReport {
    fn from(report: super::av_session::AvRecordingReport) -> Self {
        Self {
            segment_paths: report.segment_paths,
            final_output_path: report.final_output_path,
            duration_ns: report.duration_ns,
            captured_frames: report.video_captured_frames,
            accepted_frames: report.video_accepted_frames,
            encoder_input_frames: report.video_encoder_input_frames,
            encoded_frames: report.video_encoded_frames,
            dropped_by_backpressure: report.video_dropped_by_backpressure,
            audio_packet_count: Some(report.audio_encoded_packets),
            audio_pcm_frame_count: Some(report.audio_pcm_frames),
        }
    }
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
        S: RecordingFrameSource + Send,
    {
        self.start_with_factory(app_data_dir, config, move |_| Ok(source))
    }

    pub fn start_with_factory<F, S>(
        &self,
        app_data_dir: &Path,
        config: DiagnosticRecordingConfig,
        source_factory: F,
    ) -> Result<RecordingToken, RecordingManagerError>
    where
        F: FnOnce(RecordingSessionClock) -> Result<S, String> + Send + 'static,
        S: RecordingFrameSource,
    {
        let reservation = self.reserve(config.session_id.clone())?;
        let session =
            DiagnosticRecordingSession::start_with_factory(app_data_dir, config, source_factory)?;
        self.commit_start(reservation, ManagedRecordingSession::Video(session))
    }

    #[cfg(feature = "recording-opus-webm")]
    pub fn start_av_with_factories<VF, VS, AF, AS>(
        &self,
        app_data_dir: &Path,
        config: AvRecordingConfig,
        video_factory: VF,
        audio_factory: AF,
    ) -> Result<RecordingToken, RecordingManagerError>
    where
        VF: FnOnce(RecordingSessionClock) -> Result<VS, String> + Send + 'static,
        VS: RecordingFrameSource,
        AF: FnOnce(RecordingSessionClock) -> Result<AS, String> + Send + 'static,
        AS: RecordingAudioSource,
    {
        let reservation = self.reserve(config.session_id.clone())?;
        let session = AvRecordingSession::start_with_factories(
            app_data_dir,
            config,
            video_factory,
            audio_factory,
        )?;
        self.commit_start(reservation, ManagedRecordingSession::AudioVideo(session))
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
    ) -> Result<RecordingSessionReport, RecordingManagerError> {
        let (session, identity) = self.take_for_stopping(token)?;
        let result = (*session).stop();
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

    pub fn has_terminated_worker(
        &self,
        token: &RecordingToken,
    ) -> Result<bool, RecordingManagerError> {
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
        Ok(session.has_terminated_worker())
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
        session: ManagedRecordingSession,
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
    ) -> Result<(Box<ManagedRecordingSession>, Arc<()>), RecordingManagerError> {
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
    use crate::recording::segmenting::{RecordingEncoder, DEFAULT_SEGMENT_DURATION_NS};
    use std::convert::Infallible;
    #[cfg(feature = "recording-opus-webm")]
    use {
        crate::recording::audio::{AudioFormat, CapturedAudioChunk},
        crate::recording::audio_worker::RecordingAudioSource,
        crate::recording::av_session::AvRecordingConfig,
        std::collections::VecDeque,
        std::error::Error,
        std::fmt,
        std::sync::mpsc,
        std::thread,
        std::time::Duration,
    };

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
            segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
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
        assert_eq!(report.segment_paths.len(), 1);
        assert!(report.segment_paths[0].exists());
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

    struct FailedSource;

    impl RecordingFrameSource for FailedSource {
        type Error = std::io::Error;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            Err(std::io::Error::other("fixture source failed"))
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(0)
        }
    }

    #[test]
    fn detects_an_unexpected_worker_exit_before_stop_and_releases_the_slot() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new();
        let token = manager
            .start(temporary.path(), config("worker-failed"), FailedSource)
            .unwrap();

        let mut terminated = false;
        for _ in 0..100 {
            if manager.has_terminated_worker(&token).unwrap() {
                terminated = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(terminated, "失败的采集 worker 应在健康检查预算内结束");
        assert!(manager.stop(&token).is_err());
        assert_eq!(manager.status().unwrap(), RecordingManagerStatus::Idle);
    }

    #[cfg(feature = "recording-opus-webm")]
    #[derive(Debug)]
    struct AvFixtureError;

    #[cfg(feature = "recording-opus-webm")]
    impl fmt::Display for AvFixtureError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("双轨 manager fixture 已耗尽")
        }
    }

    #[cfg(feature = "recording-opus-webm")]
    impl Error for AvFixtureError {}

    #[cfg(feature = "recording-opus-webm")]
    struct AvVideoSource {
        frames: VecDeque<CapturedFrame>,
        progress: Option<mpsc::Sender<()>>,
    }

    #[cfg(feature = "recording-opus-webm")]
    impl RecordingFrameSource for AvVideoSource {
        type Error = AvFixtureError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            self.frames.pop_front().ok_or(AvFixtureError)
        }

        fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
            let frame = self.frames.pop_front();
            if frame.is_some() {
                if let Some(progress) = &self.progress {
                    let _ = progress.send(());
                }
            }
            Ok(frame)
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(400_000_000)
        }
    }

    #[cfg(feature = "recording-opus-webm")]
    struct AvAudioSource {
        chunks: VecDeque<CapturedAudioChunk>,
        progress: Option<mpsc::Sender<()>>,
    }

    #[cfg(feature = "recording-opus-webm")]
    impl RecordingAudioSource for AvAudioSource {
        type Error = AvFixtureError;

        fn capture_next_available(
            &mut self,
            timeout: Duration,
        ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
            let chunk = self.chunks.pop_front();
            if chunk.is_some() {
                if let Some(progress) = &self.progress {
                    let _ = progress.send(());
                }
            }
            if chunk.is_none() {
                thread::sleep(timeout);
            }
            Ok(chunk)
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(400_000_000)
        }
    }

    #[cfg(feature = "recording-opus-webm")]
    struct FailedAvAudioSource;

    #[cfg(feature = "recording-opus-webm")]
    impl RecordingAudioSource for FailedAvAudioSource {
        type Error = AvFixtureError;

        fn capture_next_available(
            &mut self,
            _timeout: Duration,
        ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
            Err(AvFixtureError)
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(0)
        }
    }

    #[cfg(feature = "recording-opus-webm")]
    #[test]
    fn manager_owns_and_stops_the_av_session_in_the_same_generation_slot() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new();
        let (video_progress, video_events) = mpsc::channel();
        let (audio_progress, audio_events) = mpsc::channel();
        let config = AvRecordingConfig {
            session_id: "manager-av".to_string(),
            source_id: "fixture-av".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 2,
            height: 2,
            frames_per_second: 10,
            include_cursor: false,
            audio_channels: 2,
            segment_duration_ns: 200_000_000,
        };
        let token = manager
            .start_av_with_factories(
                temporary.path(),
                config,
                move |_| {
                    Ok(AvVideoSource {
                        frames: (0..3)
                            .map(|sequence| CapturedFrame {
                                sequence,
                                captured_at_ns: sequence * 100_000_000,
                                width: 2,
                                height: 2,
                                stride: 8,
                                rgba: vec![sequence as u8; 16].into_boxed_slice(),
                            })
                            .collect(),
                        progress: Some(video_progress),
                    })
                },
                move |_| {
                    Ok(AvAudioSource {
                        chunks: (0..4)
                            .map(|sequence| CapturedAudioChunk {
                                sequence,
                                captured_at_ns: sequence * 100_000_000,
                                format: AudioFormat::normalized(2),
                                frame_count: 4_800,
                                samples: vec![0.1; 9_600].into_boxed_slice(),
                            })
                            .collect(),
                        progress: Some(audio_progress),
                    })
                },
            )
            .unwrap();
        assert_eq!(
            manager.status().unwrap(),
            RecordingManagerStatus::Recording(token.clone())
        );
        for _ in 0..3 {
            video_events
                .recv_timeout(Duration::from_secs(5))
                .expect("视频 fixture 应在预算内交付全部帧");
        }
        for _ in 0..4 {
            audio_events
                .recv_timeout(Duration::from_secs(5))
                .expect("音频 fixture 应在预算内交付全部块");
        }

        let report = manager.stop(&token).unwrap();
        assert_eq!(report.segment_paths.len(), 2);
        assert!(report.final_output_path.is_some());
        assert_eq!(report.encoded_frames, 4);
        assert!(report.audio_packet_count.unwrap() > 0);
        assert_eq!(report.audio_pcm_frame_count, Some(19_200));
        assert_eq!(manager.status().unwrap(), RecordingManagerStatus::Idle);
    }

    #[cfg(feature = "recording-opus-webm")]
    #[test]
    fn audio_failure_is_observable_and_releases_the_shared_av_slot() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new();
        let config = AvRecordingConfig {
            session_id: "manager-av-audio-failed".to_string(),
            source_id: "fixture-av".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 2,
            height: 2,
            frames_per_second: 10,
            include_cursor: false,
            audio_channels: 2,
            segment_duration_ns: 200_000_000,
        };
        let token = manager
            .start_av_with_factories(
                temporary.path(),
                config,
                |_| {
                    Ok(AvVideoSource {
                        frames: (0..4)
                            .map(|sequence| CapturedFrame {
                                sequence,
                                captured_at_ns: sequence * 100_000_000,
                                width: 2,
                                height: 2,
                                stride: 8,
                                rgba: vec![sequence as u8; 16].into_boxed_slice(),
                            })
                            .collect(),
                        progress: None,
                    })
                },
                |_| Ok(FailedAvAudioSource),
            )
            .unwrap();

        let mut terminated = false;
        for _ in 0..100 {
            if manager.has_terminated_worker(&token).unwrap() {
                terminated = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(terminated, "音频失败应终止共享双轨 pipeline");
        assert!(manager.stop(&token).is_err());
        assert_eq!(manager.status().unwrap(), RecordingManagerStatus::Idle);
    }
}
