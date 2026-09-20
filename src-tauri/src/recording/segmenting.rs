//! 周期封尾的录屏分段 writer。
//!
//! 编码线程在全局呈现时间跨过边界时立即封尾并提交当前分段；最后一个分段提交后把 journal
//! 交还会话 owner，只有采集时长与背压统计一致时才写 complete。异常与 `Drop` 保留已提交前缀，
//! 删除未提交 partial，并把清单标为 interrupted。

use super::encoder_worker::{RecordingSegmentWriter, SegmentWriterOutput};
use super::manifest::{PendingSegment, RecordingJournal};
use super::mux::avi_mjpeg::{AviMjpegError, AviMjpegOutput, AviMjpegWriter};
#[cfg(feature = "recording-vp9-prototype")]
use super::mux::vp9_webm::{Vp9WebmError, Vp9WebmOutput, Vp9WebmWriter};
use super::pipeline::{PipelineError, RecordingPipeline};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

pub(super) const DEFAULT_SEGMENT_DURATION_NS: u64 = 60_000_000_000;
const MAX_SEGMENT_DURATION_NS: u64 = 120_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecordingEncoder {
    MjpegDiagnostic {
        jpeg_quality: u8,
    },
    #[cfg(feature = "recording-vp9-prototype")]
    Vp9Prototype,
}

impl RecordingEncoder {
    pub fn manifest_descriptor(self) -> (&'static str, &'static str) {
        match self {
            Self::MjpegDiagnostic { .. } => ("mjpeg-diagnostic", "avi"),
            #[cfg(feature = "recording-vp9-prototype")]
            Self::Vp9Prototype => ("vp9-prototype", "webm"),
        }
    }

    pub fn is_valid(self) -> bool {
        match self {
            Self::MjpegDiagnostic { jpeg_quality } => (1..=100).contains(&jpeg_quality),
            #[cfg(feature = "recording-vp9-prototype")]
            Self::Vp9Prototype => true,
        }
    }
}

#[derive(Debug, Error)]
pub(super) enum SegmentedRecordingError {
    #[error("录屏周期分段配置无效")]
    InvalidConfiguration,
    #[error("录屏周期分段时间线无效")]
    InvalidTimeline,
    #[error("录屏周期分段帧数溢出")]
    FrameCountOverflow,
    #[error("录屏 journal 失败: {0}")]
    Journal(String),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error(transparent)]
    Avi(#[from] AviMjpegError),
    #[cfg(feature = "recording-vp9-prototype")]
    #[error(transparent)]
    Vp9(#[from] Vp9WebmError),
}

enum ActiveMux {
    Mjpeg(AviMjpegWriter<File>),
    #[cfg(feature = "recording-vp9-prototype")]
    Vp9(Box<Vp9WebmWriter<File>>),
}

struct FinishedMux {
    file: File,
    frame_count: u64,
}

impl ActiveMux {
    fn create(
        file: File,
        encoder: RecordingEncoder,
        width: u32,
        height: u32,
        frames_per_second: u32,
    ) -> Result<Self, SegmentedRecordingError> {
        match encoder {
            RecordingEncoder::MjpegDiagnostic { jpeg_quality } => Ok(Self::Mjpeg(
                AviMjpegWriter::new(file, width, height, frames_per_second, 1, jpeg_quality)?,
            )),
            #[cfg(feature = "recording-vp9-prototype")]
            RecordingEncoder::Vp9Prototype => Ok(Self::Vp9(Box::new(Vp9WebmWriter::new(
                file,
                width,
                height,
                frames_per_second,
                1,
            )?))),
        }
    }

    fn push_rgba(
        &mut self,
        rgba: &[u8],
        presentation_at_ns: u64,
    ) -> Result<(), SegmentedRecordingError> {
        match self {
            Self::Mjpeg(writer) => writer.push_rgba(rgba, presentation_at_ns)?,
            #[cfg(feature = "recording-vp9-prototype")]
            Self::Vp9(writer) => writer.push_rgba(rgba, presentation_at_ns)?,
        }
        Ok(())
    }

    fn finish(self, duration_ns: u64) -> Result<FinishedMux, SegmentedRecordingError> {
        match self {
            Self::Mjpeg(writer) => {
                let AviMjpegOutput {
                    writer,
                    frame_count,
                } = writer.finish_with_stats(duration_ns)?;
                Ok(FinishedMux {
                    file: writer,
                    frame_count,
                })
            }
            #[cfg(feature = "recording-vp9-prototype")]
            Self::Vp9(writer) => {
                let Vp9WebmOutput {
                    writer,
                    frame_count,
                } = (*writer).finish_with_stats(duration_ns)?;
                Ok(FinishedMux {
                    file: writer,
                    frame_count,
                })
            }
        }
    }
}

pub(super) struct PendingRecordingCompletion {
    journal: Option<RecordingJournal>,
    segment_paths: Vec<PathBuf>,
    settled: bool,
}

impl PendingRecordingCompletion {
    pub fn complete(mut self) -> Result<Vec<PathBuf>, String> {
        self.journal
            .as_mut()
            .ok_or_else(|| "录屏 journal 已经被消费".to_string())?
            .complete()?;
        self.settled = true;
        Ok(std::mem::take(&mut self.segment_paths))
    }
}

impl Drop for PendingRecordingCompletion {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        if let Some(journal) = self.journal.as_mut() {
            if let Err(error) = journal.interrupt() {
                log::warn!("回收未确认录屏清单时写入 interrupted 状态失败: {error}");
            }
        }
    }
}

pub(super) struct SegmentedRecordingWriter {
    journal: Option<RecordingJournal>,
    pending: Option<PendingSegment>,
    mux: Option<ActiveMux>,
    pipeline: Arc<RecordingPipeline>,
    encoder: RecordingEncoder,
    width: u32,
    height: u32,
    frames_per_second: u32,
    segment_duration_ns: u64,
    segment_started_at_ns: u64,
    segment_has_frame: bool,
    encoded_frames: u64,
    segment_paths: Vec<PathBuf>,
    settled: bool,
}

impl SegmentedRecordingWriter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut journal: RecordingJournal,
        pipeline: Arc<RecordingPipeline>,
        encoder: RecordingEncoder,
        width: u32,
        height: u32,
        frames_per_second: u32,
        segment_duration_ns: u64,
    ) -> Result<Self, SegmentedRecordingError> {
        if !encoder.is_valid()
            || frames_per_second == 0
            || segment_duration_ns == 0
            || segment_duration_ns > MAX_SEGMENT_DURATION_NS
        {
            let _ = journal.interrupt();
            return Err(SegmentedRecordingError::InvalidConfiguration);
        }
        let (file, pending) = match journal.begin_segment() {
            Ok(segment) => segment,
            Err(error) => {
                if let Err(cleanup_error) = journal.interrupt() {
                    log::warn!("录屏首分段创建失败后写入 interrupted 状态失败: {cleanup_error}");
                }
                return Err(SegmentedRecordingError::Journal(error));
            }
        };
        let mux = match ActiveMux::create(file, encoder, width, height, frames_per_second) {
            Ok(mux) => mux,
            Err(error) => {
                drop(pending);
                if let Err(cleanup_error) = journal.interrupt() {
                    log::warn!("录屏编码器创建失败后写入 interrupted 状态失败: {cleanup_error}");
                }
                return Err(error);
            }
        };
        Ok(Self {
            journal: Some(journal),
            pending: Some(pending),
            mux: Some(mux),
            pipeline,
            encoder,
            width,
            height,
            frames_per_second,
            segment_duration_ns,
            segment_started_at_ns: 0,
            segment_has_frame: false,
            encoded_frames: 0,
            segment_paths: Vec::new(),
            settled: false,
        })
    }

    fn rotate_at(&mut self, presentation_at_ns: u64) -> Result<(), SegmentedRecordingError> {
        let duration_ns = presentation_at_ns
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedRecordingError::InvalidTimeline)?;
        let dropped_frames = self.pipeline.stats()?.dropped_by_backpressure;
        self.commit_current(duration_ns, dropped_frames)?;

        let (file, pending) = self
            .journal
            .as_ref()
            .ok_or(SegmentedRecordingError::InvalidTimeline)?
            .begin_segment()
            .map_err(SegmentedRecordingError::Journal)?;
        let mux = ActiveMux::create(
            file,
            self.encoder,
            self.width,
            self.height,
            self.frames_per_second,
        )?;
        self.pending = Some(pending);
        self.mux = Some(mux);
        self.segment_started_at_ns = presentation_at_ns;
        self.segment_has_frame = false;
        Ok(())
    }

    fn commit_current(
        &mut self,
        duration_ns: u64,
        dropped_frames: u64,
    ) -> Result<(), SegmentedRecordingError> {
        let mux = self
            .mux
            .take()
            .ok_or(SegmentedRecordingError::InvalidTimeline)?;
        let output = mux.finish(duration_ns)?;
        let pending = self
            .pending
            .take()
            .ok_or(SegmentedRecordingError::InvalidTimeline)?;
        let path = self
            .journal
            .as_mut()
            .ok_or(SegmentedRecordingError::InvalidTimeline)?
            .commit_segment(
                pending,
                output.file,
                duration_ns,
                output.frame_count,
                dropped_frames,
            )
            .map_err(SegmentedRecordingError::Journal)?;
        self.encoded_frames = self
            .encoded_frames
            .checked_add(output.frame_count)
            .ok_or(SegmentedRecordingError::FrameCountOverflow)?;
        self.segment_paths.push(path);
        Ok(())
    }
}

impl RecordingSegmentWriter for SegmentedRecordingWriter {
    type Writer = PendingRecordingCompletion;
    type Error = SegmentedRecordingError;

    fn push_rgba(&mut self, rgba: &[u8], presentation_at_ns: u64) -> Result<(), Self::Error> {
        let elapsed = presentation_at_ns
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedRecordingError::InvalidTimeline)?;
        if self.segment_has_frame && elapsed >= self.segment_duration_ns {
            self.rotate_at(presentation_at_ns)?;
        }
        let local_presentation_ns = presentation_at_ns
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedRecordingError::InvalidTimeline)?;
        self.mux
            .as_mut()
            .ok_or(SegmentedRecordingError::InvalidTimeline)?
            .push_rgba(rgba, local_presentation_ns)?;
        self.segment_has_frame = true;
        Ok(())
    }

    fn finish_with_stats(
        mut self,
        duration_ns: u64,
    ) -> Result<SegmentWriterOutput<Self::Writer>, Self::Error> {
        let local_duration_ns = duration_ns
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedRecordingError::InvalidTimeline)?;
        let dropped_frames = self.pipeline.stats()?.dropped_by_backpressure;
        self.commit_current(local_duration_ns, dropped_frames)?;
        let completion = PendingRecordingCompletion {
            journal: self.journal.take(),
            segment_paths: std::mem::take(&mut self.segment_paths),
            settled: false,
        };
        self.settled = true;
        Ok(SegmentWriterOutput {
            writer: completion,
            frame_count: self.encoded_frames,
        })
    }
}

impl Drop for SegmentedRecordingWriter {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        drop(self.mux.take());
        drop(self.pending.take());
        if let Some(journal) = self.journal.as_mut() {
            if let Err(error) = journal.interrupt() {
                log::warn!("回收录屏周期分段时写入 interrupted 状态失败: {error}");
            }
        }
    }
}
