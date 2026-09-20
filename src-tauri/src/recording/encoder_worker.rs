//! 有界录屏 pipeline 到诊断分段编码器的独立消费线程。
//!
//! 消费者先排空已接受帧，再用采集线程提供的最终有效时长封尾。编码、容器或队列失败都会中止
//! pipeline，使仍在运行的采集线程尽快停止；`Drop` 同样中止并回收线程，禁止留下永久等待者。

use super::mux::avi_mjpeg::{AviMjpegError, AviMjpegWriter};
#[cfg(feature = "recording-vp9-prototype")]
use super::mux::vp9_webm::{Vp9WebmError, Vp9WebmWriter};
use super::pipeline::{PipelineDrain, PipelineError, RecordingPipeline};
use std::error::Error as StdError;
use std::io::{Seek, Write};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum EncoderWorkerError<E>
where
    E: StdError + Send + 'static,
{
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error("录屏分段编码失败: {0}")]
    Mux(E),
    #[error("无法启动录屏编码线程: {0}")]
    ThreadSpawn(String),
    #[error("录屏编码线程异常退出")]
    ThreadPanicked,
}

pub(super) struct EncoderReport<W> {
    pub writer: W,
    pub input_frames: u64,
    pub encoded_frames: u64,
    pub duration_ns: u64,
}

pub(super) struct SegmentWriterOutput<W> {
    pub writer: W,
    pub frame_count: u64,
}

pub(super) trait RecordingSegmentWriter: Send + 'static {
    type Writer: Send + 'static;
    type Error: StdError + Send + 'static;

    fn push_rgba(&mut self, rgba: &[u8], presentation_at_ns: u64) -> Result<(), Self::Error>;

    fn finish_with_stats(
        self,
        duration_ns: u64,
    ) -> Result<SegmentWriterOutput<Self::Writer>, Self::Error>;
}

type EncoderJoinResult<M> = Result<
    EncoderReport<<M as RecordingSegmentWriter>::Writer>,
    EncoderWorkerError<<M as RecordingSegmentWriter>::Error>,
>;

impl<W> RecordingSegmentWriter for AviMjpegWriter<W>
where
    W: Write + Seek + Send + 'static,
{
    type Writer = W;
    type Error = AviMjpegError;

    fn push_rgba(&mut self, rgba: &[u8], presentation_at_ns: u64) -> Result<(), Self::Error> {
        AviMjpegWriter::push_rgba(self, rgba, presentation_at_ns)
    }

    fn finish_with_stats(
        self,
        duration_ns: u64,
    ) -> Result<SegmentWriterOutput<Self::Writer>, Self::Error> {
        let output = AviMjpegWriter::finish_with_stats(self, duration_ns)?;
        Ok(SegmentWriterOutput {
            writer: output.writer,
            frame_count: output.frame_count,
        })
    }
}

#[cfg(feature = "recording-vp9-prototype")]
impl<W> RecordingSegmentWriter for Vp9WebmWriter<W>
where
    W: Write + Seek + Send + 'static,
{
    type Writer = W;
    type Error = Vp9WebmError;

    fn push_rgba(&mut self, rgba: &[u8], presentation_at_ns: u64) -> Result<(), Self::Error> {
        Vp9WebmWriter::push_rgba(self, rgba, presentation_at_ns)
    }

    fn finish_with_stats(
        self,
        duration_ns: u64,
    ) -> Result<SegmentWriterOutput<Self::Writer>, Self::Error> {
        let output = Vp9WebmWriter::finish_with_stats(self, duration_ns)?;
        Ok(SegmentWriterOutput {
            writer: output.writer,
            frame_count: output.frame_count,
        })
    }
}

pub(super) struct EncoderWorker<M>
where
    M: RecordingSegmentWriter,
{
    pipeline: Arc<RecordingPipeline>,
    join: Option<JoinHandle<EncoderJoinResult<M>>>,
}

impl<M> EncoderWorker<M>
where
    M: RecordingSegmentWriter,
{
    pub fn spawn(
        writer: M,
        pipeline: Arc<RecordingPipeline>,
    ) -> Result<Self, EncoderWorkerError<M::Error>> {
        let worker_pipeline = Arc::clone(&pipeline);
        let join = thread::Builder::new()
            .name("clippy-recording-encoder".to_string())
            .spawn(move || encode_until_terminal(writer, &worker_pipeline))
            .map_err(|error| {
                let _ = pipeline.abort();
                EncoderWorkerError::ThreadSpawn(error.to_string())
            })?;
        Ok(Self {
            pipeline,
            join: Some(join),
        })
    }

    pub fn wait(mut self) -> Result<EncoderReport<M::Writer>, EncoderWorkerError<M::Error>> {
        self.join_inner()
    }

    fn join_inner(&mut self) -> Result<EncoderReport<M::Writer>, EncoderWorkerError<M::Error>> {
        let Some(join) = self.join.take() else {
            return Err(EncoderWorkerError::ThreadPanicked);
        };
        join.join()
            .map_err(|_| EncoderWorkerError::ThreadPanicked)?
    }
}

impl<M> Drop for EncoderWorker<M>
where
    M: RecordingSegmentWriter,
{
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = self.pipeline.abort();
            let _ = join.join();
        }
    }
}

fn encode_until_terminal<M>(
    mut writer: M,
    pipeline: &RecordingPipeline,
) -> Result<EncoderReport<M::Writer>, EncoderWorkerError<M::Error>>
where
    M: RecordingSegmentWriter,
{
    let mut abort_guard = EncoderAbortGuard::new(pipeline);
    let mut input_frames = 0_u64;
    loop {
        match pipeline.pop_wait()? {
            PipelineDrain::Frame(queued) => {
                writer
                    .push_rgba(&queued.frame.rgba, queued.presentation_at_ns)
                    .map_err(EncoderWorkerError::Mux)?;
                input_frames = input_frames.saturating_add(1);
            }
            PipelineDrain::Finished { duration_ns } => {
                let output = writer
                    .finish_with_stats(duration_ns)
                    .map_err(EncoderWorkerError::Mux)?;
                abort_guard.disarm();
                return Ok(EncoderReport {
                    writer: output.writer,
                    input_frames,
                    encoded_frames: output.frame_count,
                    duration_ns,
                });
            }
        }
    }
}

pub(super) type DiagnosticEncoderError = EncoderWorkerError<AviMjpegError>;
pub(super) type DiagnosticEncoderReport<W> = EncoderReport<W>;
pub(super) type DiagnosticEncoderWorker<W> = EncoderWorker<AviMjpegWriter<W>>;

#[cfg(feature = "recording-vp9-prototype")]
pub(super) type Vp9EncoderError = EncoderWorkerError<Vp9WebmError>;
#[cfg(feature = "recording-vp9-prototype")]
pub(super) type Vp9EncoderWorker<W> = EncoderWorker<Vp9WebmWriter<W>>;

struct EncoderAbortGuard<'a> {
    pipeline: &'a RecordingPipeline,
    armed: bool,
}

impl<'a> EncoderAbortGuard<'a> {
    fn new(pipeline: &'a RecordingPipeline) -> Self {
        Self {
            pipeline,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for EncoderAbortGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.pipeline.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::frame::CapturedFrame;
    use crate::recording::worker::{CaptureWorker, RecordingFrameSource};
    use std::convert::Infallible;
    use std::io::Cursor;

    fn frame(sequence: u64, captured_at_ns: u64, marker: u8) -> CapturedFrame {
        CapturedFrame {
            sequence,
            captured_at_ns,
            width: 2,
            height: 2,
            stride: 8,
            rgba: vec![marker; 16].into_boxed_slice(),
        }
    }

    #[cfg(feature = "recording-vp9-prototype")]
    fn vp9_frame(sequence: u64, captured_at_ns: u64, marker: u8) -> CapturedFrame {
        let mut rgba = vec![marker; 64 * 48 * 4];
        for alpha in rgba.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        CapturedFrame {
            sequence,
            captured_at_ns,
            width: 64,
            height: 48,
            stride: 64 * 4,
            rgba: rgba.into_boxed_slice(),
        }
    }

    fn writer(width: u32, height: u32) -> AviMjpegWriter<Cursor<Vec<u8>>> {
        AviMjpegWriter::new(Cursor::new(Vec::new()), width, height, 10, 1, 85).unwrap()
    }

    #[test]
    fn consumes_frames_and_finishes_a_playable_container_from_pipeline_duration() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = DiagnosticEncoderWorker::spawn(writer(2, 2), Arc::clone(&pipeline)).unwrap();
        pipeline.push(frame(0, 100, 10)).unwrap();
        pipeline.push(frame(1, 100_000_100, 20)).unwrap();
        assert_eq!(pipeline.finish(200_000_100).unwrap(), 200_000_000);

        let report = worker.wait().unwrap();
        assert_eq!(report.input_frames, 2);
        assert_eq!(report.encoded_frames, 2);
        assert_eq!(report.duration_ns, 200_000_000);
        let bytes = report.writer.into_inner();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"AVI ");
        let index = bytes
            .windows(4)
            .position(|window| window == b"idx1")
            .expect("编码线程必须正常封尾索引");
        assert_eq!(
            u32::from_le_bytes(bytes[index + 4..index + 8].try_into().unwrap()),
            32
        );
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn generic_worker_finishes_vp9_webm_with_the_pipeline_duration() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let writer = Vp9WebmWriter::new(Cursor::new(Vec::new()), 64, 48, 10, 1).unwrap();
        let worker = Vp9EncoderWorker::spawn(writer, Arc::clone(&pipeline)).unwrap();
        pipeline.push(vp9_frame(0, 100, 10)).unwrap();
        pipeline.push(vp9_frame(1, 100_000_100, 220)).unwrap();
        assert_eq!(pipeline.finish(200_000_100).unwrap(), 200_000_000);

        let report = worker.wait().unwrap();
        assert_eq!(report.input_frames, 2);
        assert_eq!(report.encoded_frames, 2);
        assert_eq!(report.duration_ns, 200_000_000);
        assert_eq!(&report.writer.into_inner()[0..4], [0x1a, 0x45, 0xdf, 0xa3]);
    }

    #[test]
    fn drains_bounded_prefix_before_observing_finish() {
        let pipeline = Arc::new(RecordingPipeline::default());
        for sequence in 0..4 {
            pipeline
                .push(frame(
                    sequence,
                    100 + sequence * 100_000_000,
                    sequence as u8,
                ))
                .unwrap();
        }
        pipeline.finish(400_000_100).unwrap();
        let worker = DiagnosticEncoderWorker::spawn(writer(2, 2), Arc::clone(&pipeline)).unwrap();
        let report = worker.wait().unwrap();
        assert_eq!(report.input_frames, 3);
        assert_eq!(report.encoded_frames, 4);
        assert_eq!(pipeline.stats().unwrap().dropped_by_backpressure, 1);
        assert_eq!(report.duration_ns, 400_000_000);
    }

    #[test]
    fn aborted_pipeline_drains_prefix_then_returns_the_terminal_error() {
        let pipeline = Arc::new(RecordingPipeline::default());
        pipeline.push(frame(0, 100, 0)).unwrap();
        pipeline.abort().unwrap();
        let worker = DiagnosticEncoderWorker::spawn(writer(2, 2), Arc::clone(&pipeline)).unwrap();
        assert!(matches!(
            worker.wait(),
            Err(DiagnosticEncoderError::Pipeline(PipelineError::Aborted))
        ));
        assert_eq!(pipeline.stats().unwrap().queued_frames, 0);
    }

    #[test]
    fn encoder_failure_aborts_pipeline_and_preserves_unconsumed_frames() {
        let pipeline = Arc::new(RecordingPipeline::default());
        pipeline.push(frame(0, 100, 0)).unwrap();
        pipeline.push(frame(1, 110, 1)).unwrap();
        let worker = DiagnosticEncoderWorker::spawn(writer(1, 1), Arc::clone(&pipeline)).unwrap();
        assert!(matches!(
            worker.wait(),
            Err(DiagnosticEncoderError::Mux(AviMjpegError::InvalidFrame))
        ));
        while pipeline.pop().unwrap().is_some() {}
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }

    #[test]
    fn dropping_worker_aborts_pipeline_and_joins_waiter() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = DiagnosticEncoderWorker::spawn(writer(2, 2), Arc::clone(&pipeline)).unwrap();
        drop(worker);
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }

    struct EndToEndSource {
        sequence: u64,
        timestamp_ns: u64,
    }

    impl RecordingFrameSource for EndToEndSource {
        type Error = Infallible;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            let frame = frame(self.sequence, self.timestamp_ns, self.sequence as u8);
            self.sequence += 1;
            self.timestamp_ns += 100_000_000;
            Ok(frame)
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            let timestamp_ns = self.timestamp_ns;
            self.timestamp_ns += 1;
            Ok(timestamp_ns)
        }
    }

    #[test]
    fn capture_pipeline_and_encoder_share_one_final_duration() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let encoder = DiagnosticEncoderWorker::spawn(writer(2, 2), Arc::clone(&pipeline)).unwrap();
        let capture = CaptureWorker::spawn(
            EndToEndSource {
                sequence: 0,
                timestamp_ns: 100,
            },
            Arc::clone(&pipeline),
            120,
        )
        .unwrap();

        let capture_report = capture.stop().unwrap();
        let encoder_report = encoder.wait().unwrap();
        assert!(capture_report.captured_frames >= 1);
        assert_eq!(capture_report.duration_ns, Some(encoder_report.duration_ns));
        assert!(encoder_report.input_frames >= 1);
        assert_eq!(&encoder_report.writer.into_inner()[0..4], b"RIFF");
    }
}
