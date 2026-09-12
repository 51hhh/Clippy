use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fake_program(body: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tesseract-fake");
    let pid = dir.path().join("pid");
    std::fs::write(&path, format!("#!/usr/bin/env python3\nimport os, sys, time\nopen({:?}, 'w').write(str(os.getpid()))\n{body}\n", pid)).unwrap();
    (dir, path, pid)
}

async fn run_fake_with_timeout(
    path: &Path,
    png: &[u8],
    timeout: Duration,
    stdout: usize,
    stderr: usize,
) -> OcrResult {
    // 通过稳定解释器读取临时脚本，避免并行进程 fork 继承临时写 fd 导致 ETXTBSY。
    let mut command = tokio::process::Command::new("python3");
    command.arg(path);
    run_recognition_process(command, png, timeout, stdout, stderr).await
}

fn assert_reaped(pid: &Path) {
    #[cfg(target_os = "linux")]
    if let Ok(id) = std::fs::read_to_string(pid) {
        assert!(
            !Path::new(&format!("/proc/{id}")).exists(),
            "超时后不能遗留存活/僵尸子进程"
        );
    }
    let _ = pid;
}

#[tokio::test]
async fn hanging_and_nonreading_children_timeout_are_reaped_and_release_the_permit() {
    let runtime = Box::leak(Box::new(OcrRuntime::new(1)));
    for (body, size) in [
        ("sys.stdin.buffer.read(); time.sleep(60)", 16),
        ("time.sleep(60)", 2 * 1024 * 1024),
    ] {
        let (_dir, executable, pid) = fake_program(body);
        let start = Instant::now();
        let result = runtime
            .run_clip(42, move || async move {
                run_fake_with_timeout(
                    &executable,
                    &vec![42; size],
                    Duration::from_millis(250),
                    4096,
                    4096,
                )
                .await
            })
            .await;
        assert!(result.unwrap_err().contains("超时"));
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_reaped(&pid);
        assert_eq!(
            runtime
                .run_clip(43, || async { Ok("recovered".into()) })
                .await
                .unwrap(),
            "recovered"
        );
        assert!(runtime.in_flight.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn stdout_and_stderr_limits_kill_children_and_valid_utf8_survives() {
    for channel in ["stdout", "stderr"] {
        let (_dir, executable, pid) = fake_program(&format!(
            "sys.{channel}.buffer.write(b'x' * 1000000); sys.{channel}.flush(); time.sleep(60)"
        ));
        let result =
            run_fake_with_timeout(&executable, b"", Duration::from_secs(2), 4096, 4096).await;
        let error = result.unwrap_err();
        assert!(error.contains("上限"), "{channel}: {error}");
        assert_reaped(&pid);
    }
    let (_dir, executable, pid) =
        fake_program("sys.stdin.buffer.read(); print('  完整 OCR text  ')");
    assert_eq!(
        run_fake_with_timeout(&executable, b"image", Duration::from_secs(2), 4096, 4096)
            .await
            .unwrap(),
        "完整 OCR text"
    );
    assert_reaped(&pid);
}

#[test]
fn hung_probe_is_reaped_and_cold_probe_is_single_flight() {
    let (_dir, path, pid) = fake_program("time.sleep(60)");
    let mut command = Command::new("python3");
    command.arg(&path);
    assert!(!probe_command_with_timeout(
        command,
        Duration::from_millis(200)
    ));
    assert_reaped(&pid);
    let cache = Arc::new(ExecutableCache::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(5));
    let threads: Vec<_> = (0..5)
        .map(|_| {
            let cache = cache.clone();
            let calls = calls.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                cache.resolve_with(|| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(25));
                    Some(PathBuf::from("one"))
                })
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), Some(PathBuf::from("one")));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn queued_lazy_jobs_do_not_load_images_and_abandoned_waiters_do_not_cancel_others() {
    let runtime = Box::leak(Box::new(OcrRuntime::new(1)));
    let permit = runtime.permits.acquire().await.unwrap();
    let loads = Arc::new(AtomicUsize::new(0));
    let start = |id, loads: Arc<AtomicUsize>| {
        runtime.run_clip(id, move || async move {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok("shared".into())
        })
    };
    let first = tokio::spawn(start(1, loads.clone()));
    let shared = tokio::spawn(start(1, loads.clone()));
    let abandoned = tokio::spawn(start(2, loads.clone()));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(loads.load(Ordering::SeqCst), 0);
    first.abort();
    abandoned.abort();
    let _ = first.await;
    let _ = abandoned.await;
    drop(permit);
    assert_eq!(shared.await.unwrap().unwrap(), "shared");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert!(runtime.in_flight.lock().unwrap().is_empty());
}

#[tokio::test]
async fn queue_has_a_hard_admission_limit() {
    let runtime = Box::leak(Box::new(OcrRuntime::new(1)));
    let permit = runtime.permits.acquire().await.unwrap();
    let mut jobs = Vec::new();
    for id in 0..3 {
        jobs.push(tokio::spawn(
            runtime.run_clip(id, || async { Ok("ok".into()) }),
        ));
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(runtime
        .run_clip(99, || async { Ok("must not run".into()) })
        .await
        .unwrap_err()
        .contains("队列已满"));
    drop(permit);
    for job in jobs {
        assert_eq!(job.await.unwrap().unwrap(), "ok");
    }
}
