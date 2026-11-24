use super::segmenting::{RecordingEncoder, DEFAULT_SEGMENT_DURATION_NS};
use super::session::DiagnosticRecordingConfig;
use super::session::DiagnosticRecordingSession;
use super::test_artifacts as tempfile;
use clippy_media::recording::{frame::CapturedFrame, worker::RecordingFrameSource};
use std::{sync::mpsc, time::Duration};

#[cfg(target_os = "linux")]
#[test]
#[ignore = "由 ci-local.sh 在隔离 Xvfb 中显式运行"]
fn x11_source_records_a_complete_private_avi_session() {
    use crate::capture::RecordingCaptureSpec;
    use crate::recording::platform::x11::X11RegionFrameSourcePlan;
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    let (connection, screen_number) = RustConnection::connect(None).expect("连接测试 X11");
    let root = connection.setup().roots[screen_number].root;
    connection
        .randr_query_version(1, 5)
        .expect("请求 RandR 版本")
        .reply()
        .expect("读取 RandR 版本");
    let monitors = connection
        .randr_get_monitors(root, true)
        .expect("请求 RandR 显示器")
        .reply()
        .expect("读取 RandR 显示器");
    let monitor = monitors
        .monitors
        .iter()
        .find(|monitor| !monitor.outputs.is_empty())
        .expect("Xvfb 至少暴露一个 RandR output");
    let width = u32::from(monitor.width).min(64);
    let height = u32::from(monitor.height).min(64);
    let plan = X11RegionFrameSourcePlan::prepare(RecordingCaptureSpec {
        monitor_id: monitor.outputs[0],
        monitor_pixel_width: u32::from(monitor.width),
        monitor_pixel_height: u32::from(monitor.height),
        crop_left: 0,
        crop_top: 0,
        crop_width: width,
        crop_height: height,
    })
    .expect("准备 X11 录屏帧源");
    let descriptor = plan.descriptor().clone();
    assert_eq!(descriptor.physical_x, i32::from(monitor.x));
    assert_eq!(descriptor.physical_y, i32::from(monitor.y));
    assert_eq!((descriptor.width, descriptor.height), (width, height));
    struct Probe<S>(S, mpsc::SyncSender<()>);
    impl<S: RecordingFrameSource> RecordingFrameSource for Probe<S> {
        type Error = S::Error;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            let frame = self.0.capture_next()?;
            let _ = self.1.try_send(());
            Ok(frame)
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            self.0.control_timestamp_ns()
        }
    }
    let (produce, observe) = mpsc::sync_channel(3);
    let temporary = tempfile::tempdir().unwrap();
    let session = DiagnosticRecordingSession::start_with_factory(
        temporary.path(),
        DiagnosticRecordingConfig {
            session_id: "x11-e2e".to_string(),
            source_id: descriptor.source_id,
            physical_x: descriptor.physical_x,
            physical_y: descriptor.physical_y,
            width: descriptor.width,
            height: descriptor.height,
            frames_per_second: 30,
            include_cursor: true,
            encoder: RecordingEncoder::MjpegDiagnostic { jpeg_quality: 85 },
            segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
        },
        move |clock| {
            let source = plan.connect(clock).map_err(|error| error.to_string())?;
            Ok(Probe(source, produce))
        },
    )
    .expect("启动 X11 诊断录屏");
    for _ in 0..3 {
        observe
            .recv_timeout(Duration::from_secs(5))
            .expect("获取 X11 录屏像素");
    }
    let report = session.stop().expect("停止并提交 X11 诊断录屏");
    assert!(report.captured_frames >= 2);
    assert!(report.encoded_frames >= 1);
    assert_eq!(report.segment_paths.len(), 1);
    assert_eq!(
        &std::fs::read(&report.segment_paths[0]).unwrap()[0..4],
        b"RIFF"
    );
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            report.segment_paths[0]
                .parent()
                .unwrap()
                .join("manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["state"], "complete");
    assert_eq!(manifest["segments"][0]["frameCount"], report.encoded_frames);
    assert!(crate::private_files::is_private(&report.segment_paths[0]));
}
