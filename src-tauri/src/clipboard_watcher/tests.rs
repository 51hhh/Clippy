use super::*;
use std::borrow::Cow;

fn snapshot(kind: &str, text: &str) -> ClipboardSnapshot {
    match kind {
        "html" => ClipboardSnapshot::Html {
            html: format!("<b>{text}</b>"),
            text: text.to_owned(),
        },
        "image" => ClipboardSnapshot::Image(arboard::ImageData {
            width: 1,
            height: 1,
            bytes: Cow::Owned(vec![text.as_bytes()[0], 2, 3, 255]),
        }),
        _ => ClipboardSnapshot::Text(text.to_owned()),
    }
}

#[test]
fn every_snapshot_retries_real_database_failure_and_emits_only_one_success() {
    for kind in ["text", "html", "image"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watcher.db");
        let storage = StorageEngine::new(&path).unwrap();
        let faults = rusqlite::Connection::open(path).unwrap();
        faults.execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON clips BEGIN SELECT RAISE(ABORT, 'temporary failure'); END;").unwrap();
        let mut state = PollState::default();
        let mut rejected = None;
        let now = Instant::now();
        let mut successes = 0;
        let mut failures = 0;
        for elapsed in [0, 100, 500, 1000, 2000] {
            let time = now + Duration::from_millis(elapsed);
            if let Some(item) = prepare_snapshot(
                snapshot(kind, "payload"),
                &[],
                &mut state,
                &mut rejected,
                time,
            ) {
                match storage.insert_clip(
                    &item.kind,
                    item.text.as_deref(),
                    item.html.as_deref(),
                    item.png.as_deref(),
                    &item.hash,
                    item.byte_size,
                    false,
                ) {
                    Ok(_) => {
                        state.settle();
                        successes += 1;
                    }
                    Err(_) => {
                        state.failed(time);
                        failures += 1;
                        faults.execute_batch("DROP TRIGGER fail_insert;").unwrap();
                    }
                }
            }
        }
        assert_eq!((failures, successes), (1, 1), "{kind}");
        assert_eq!(storage.get_clips(None, false, 0, 10).unwrap().len(), 1);
    }
}

#[test]
fn text_html_and_image_suppression_cover_successive_polls_and_then_external_content() {
    for kind in ["text", "html", "image"] {
        let mut probe = PollState::default();
        let mut rejected = None;
        let now = Instant::now();
        let item = prepare_snapshot(
            snapshot(kind, "result"),
            &[],
            &mut probe,
            &mut rejected,
            now,
        )
        .unwrap();
        let mut state = PollState::default();
        for index in 0..3 {
            let suppression = if index == 0 {
                vec![item.hash.clone()]
            } else {
                vec![]
            };
            assert!(prepare_snapshot(
                snapshot(kind, "result"),
                &suppression,
                &mut state,
                &mut rejected,
                now
            )
            .is_none());
        }
        assert!(prepare_snapshot(
            snapshot(kind, "external"),
            &[],
            &mut state,
            &mut rejected,
            now
        )
        .is_some());
        state.settle();
        assert!(prepare_snapshot(
            snapshot(kind, "result"),
            &[],
            &mut state,
            &mut rejected,
            now
        )
        .is_some());
    }
}

#[test]
fn rejected_image_does_not_suppress_the_previous_valid_image() {
    let mut state = PollState::default();
    let mut rejected = None;
    let now = Instant::now();
    assert!(prepare_snapshot(
        snapshot("image", "valid"),
        &[],
        &mut state,
        &mut rejected,
        now
    )
    .is_some());
    state.settle();
    let invalid = ClipboardSnapshot::Image(arboard::ImageData {
        width: 0,
        height: 1,
        bytes: Cow::Owned(vec![]),
    });
    assert!(prepare_snapshot(invalid, &[], &mut state, &mut rejected, now).is_none());
    assert!(prepare_snapshot(
        snapshot("image", "valid"),
        &[],
        &mut state,
        &mut rejected,
        now
    )
    .is_some());
}

#[test]
fn failed_programmatic_write_does_not_publish_a_stale_suppression() {
    let watcher = ClipboardWatcher::new();
    watcher
        .write_suppressed(vec!["previous".into()], || Ok(()))
        .unwrap();
    assert!(watcher
        .write_suppressed(vec!["failed".into()], || Err("busy".into()))
        .is_err());
    assert_eq!(
        watcher.suppressed_hashes.lock().unwrap().hashes,
        vec!["previous"]
    );
    watcher
        .write_suppressed(vec!["next".into()], || Ok(()))
        .unwrap();
    assert_eq!(
        watcher.suppressed_hashes.lock().unwrap().hashes,
        vec!["next"]
    );
}

#[test]
fn writer_and_reader_share_one_snapshot_boundary_without_polling_under_lock() {
    use std::sync::mpsc;
    let watcher = Arc::new(ClipboardWatcher::new());
    let writer = watcher.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writing = thread::spawn(move || {
        writer.write_suppressed(vec!["new".into()], || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    entered_rx.recv().unwrap();
    // 在实际写入完成与抑制登记之间，读取快照不能穿过同一个门。
    assert!(watcher.suppressed_hashes.try_lock().is_err());
    release_tx.send(()).unwrap();
    writing.join().unwrap().unwrap();
    assert_eq!(
        watcher.suppressed_hashes.lock().unwrap().hashes,
        vec!["new"]
    );
}

#[test]
fn slow_clipboard_read_never_blocks_a_programmatic_write_and_stale_snapshot_is_rejected() {
    use std::sync::mpsc;
    let watcher = Arc::new(ClipboardWatcher::new());
    let reader = watcher.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let reading = thread::spawn(move || {
        let generation = reader.suppressed_hashes.lock().unwrap().generation;
        started_tx.send(()).unwrap();
        finish_rx.recv().unwrap(); // 模拟任意时长的系统读取，绝不持锁。
        reader.suppressed_hashes.lock().unwrap().generation == generation
    });
    started_rx.recv().unwrap();
    watcher
        .write_suppressed(vec!["new".into()], || Ok(()))
        .unwrap();
    finish_tx.send(()).unwrap();
    assert!(!reading.join().unwrap());
    assert_eq!(
        watcher.suppressed_hashes.lock().unwrap().hashes,
        vec!["new"]
    );
}

#[test]
fn a_new_external_selection_event_can_recopy_the_same_suppressed_image() {
    let now = Instant::now();
    let mut state = PollState::default();
    let mut rejected = None;
    let image = prepare_snapshot(
        snapshot("image", "image"),
        &[],
        &mut state,
        &mut rejected,
        now,
    )
    .unwrap();
    state.reset();
    assert!(prepare_snapshot(
        snapshot("image", "image"),
        std::slice::from_ref(&image.hash),
        &mut state,
        &mut rejected,
        now
    )
    .is_none());
    assert!(prepare_snapshot(
        snapshot("image", "image"),
        &[],
        &mut state,
        &mut rejected,
        now
    )
    .is_none());
    // 真实 XFixes 新事件即使 owner/像素相同也会重置观察代次；一次性写入抑制已消费。
    state.reset();
    assert!(prepare_snapshot(
        snapshot("image", "image"),
        &[],
        &mut state,
        &mut rejected,
        now
    )
    .is_some());
}

/// 必须显式在独立 Xvfb 中运行，普通 cargo test 不接触桌面剪贴板。
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a private Xvfb and CLIPPY_TEST_X11_ISOLATED=1"]
fn x11_internal_writes_are_settled_before_read_and_external_recopies_are_kept() {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;

    assert_eq!(
        std::env::var("CLIPPY_TEST_X11_ISOLATED").as_deref(),
        Ok("1")
    );
    if std::env::var_os("CLIPPY_TEST_X11_PROVIDER").is_some() {
        // 独立进程有独立 arboard selection owner，不能用同进程的另一个 Clipboard 冒充外部复制。
        let mut clipboard = Clipboard::new().unwrap();
        for line in std::io::stdin().lock().lines() {
            clipboard.set_text(line.unwrap()).unwrap();
            println!("CLIPPY_X11_WRITTEN");
            std::io::stdout().flush().unwrap();
        }
        return;
    }

    let watcher = ClipboardWatcher::new();
    let mut clipboard = Clipboard::new().unwrap();
    // 控制服务器暂不处理其他连接。旧 flush 会在 ownership 请求尚未执行时返回；
    // 正确的写完成屏障必须等服务器释放。这是协议故障注入，不靠睡眠修正产品时序。
    {
        use x11rb::protocol::xproto::ConnectionExt as _;
        let mut writing_clipboard = Clipboard::new().unwrap();
        let (server, _) = x11rb::connect(None).unwrap();
        server.grab_server().unwrap().check().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (written_tx, written_rx) = mpsc::channel();
        let writing = thread::spawn(move || {
            entered_tx.send(()).unwrap();
            let result = writing_clipboard.set_text("synthetic barrier probe");
            let _ = written_tx.send(result);
        });
        entered_rx.recv().unwrap();
        let premature = written_rx.recv_timeout(Duration::from_millis(100));
        // 即使断言失败，也先释放私有服务器并回收写线程。
        server.ungrab_server().unwrap().check().unwrap();
        writing.join().unwrap();
        assert!(matches!(premature, Err(mpsc::RecvTimeoutError::Timeout)));
        written_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
    }
    let mut changes = changes::ChangeMonitor::new();
    assert!(changes.poll().1, "隔离 X server 必须支持 XFixes");
    let mut state = PollState::default();
    let mut rejected = None;
    let mut last_text = String::new();
    for index in 0..1_000 {
        last_text = format!("synthetic internal result {index}");
        let hash = compute_hash(last_text.as_bytes());
        watcher
            .write_suppressed(vec![hash], || clipboard_set_text_with_retry(&last_text))
            .unwrap();
        // 关键回归：实际写连接的屏障使本次事件在读取/消费抑制之前可见。
        // 旧 arboard 只有 flush，真实 Xvfb 下会出现 false → 读到新值 → true。
        assert_eq!(changes.poll(), (true, true), "写入事件不能迟到: {index}");
        state.reset();
        let hashes = std::mem::take(&mut watcher.suppressed_hashes.lock().unwrap().hashes);
        assert!(prepare_snapshot(
            ClipboardSnapshot::read(&mut clipboard).unwrap(),
            &hashes,
            &mut state,
            &mut rejected,
            Instant::now(),
        )
        .is_none());
        // 抑制只消费一次，之后多轮仍不得把同一内部结果放入历史。
        for _ in 0..2 {
            assert_eq!(changes.poll(), (false, true));
            assert!(prepare_snapshot(
                ClipboardSnapshot::read(&mut clipboard).unwrap(),
                &[],
                &mut state,
                &mut rejected,
                Instant::now(),
            )
            .is_none());
        }
    }

    // 同一个外部进程连续两次复制相同内容：第一次允许入库，第二次必须重新置顶。
    struct Provider(std::process::Child);
    impl Drop for Provider {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut provider = Provider(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "clipboard_watcher::tests::x11_internal_writes_are_settled_before_read_and_external_recopies_are_kept",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env("CLIPPY_TEST_X11_PROVIDER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let output = provider.0.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::BufReader::new(output)
            .lines()
            .map_while(Result::ok)
        {
            if line == "CLIPPY_X11_WRITTEN" {
                let _ = ready_tx.send(());
            }
        }
    });
    let storage = StorageEngine::new_in_memory().unwrap();
    let mut first_id = None;
    for _ in 0..2 {
        let input = provider.0.stdin.as_mut().unwrap();
        writeln!(input, "{last_text}").unwrap();
        input.flush().unwrap();
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(changes.poll(), (true, true));
        state.reset();
        let item = prepare_snapshot(
            ClipboardSnapshot::read(&mut clipboard).unwrap(),
            &[],
            &mut state,
            &mut rejected,
            Instant::now(),
        )
        .expect("合法外部重复制不能被内部抑制吞掉");
        let clip = storage
            .insert_clip(
                &item.kind,
                item.text.as_deref(),
                item.html.as_deref(),
                item.png.as_deref(),
                &item.hash,
                item.byte_size,
                false,
            )
            .unwrap();
        if let Some(id) = first_id {
            assert_eq!(clip.id, id);
        }
        first_id = Some(clip.id);
        assert_eq!(
            storage.get_clips(None, false, 0, 10).unwrap()[0].id,
            clip.id
        );
        state.settle();
        storage
            .insert_clip(
                &ContentType::Text,
                Some("between"),
                None,
                None,
                "between",
                7,
                false,
            )
            .unwrap();
    }
}

#[test]
fn unchanged_selection_notification_does_not_block_due_database_retries() {
    let now = Instant::now();
    let mut state = PollState::default();
    let mut rejected = None;
    assert!(prepare_snapshot(
        snapshot("image", "image"),
        &[],
        &mut state,
        &mut rejected,
        now
    )
    .is_some());
    state.failed(now);
    assert!(!state.needs_retry(now + Duration::from_millis(100)));
    assert!(state.needs_retry(now + Duration::from_millis(500)));
    assert!(prepare_snapshot(
        snapshot("image", "image"),
        &[],
        &mut state,
        &mut rejected,
        now + Duration::from_millis(500)
    )
    .is_some());
    state.settle();
    assert!(!state.needs_retry(now + Duration::from_secs(1)));
}

#[test]
fn last_pixel_change_is_not_lost_by_the_full_image_fingerprint() {
    let now = Instant::now();
    let mut state = PollState::default();
    let mut rejected = None;
    let mut pixels = vec![255; 32 * 32 * 4];
    let make = |bytes| {
        ClipboardSnapshot::Image(arboard::ImageData {
            width: 32,
            height: 32,
            bytes: Cow::Owned(bytes),
        })
    };
    let before =
        prepare_snapshot(make(pixels.clone()), &[], &mut state, &mut rejected, now).unwrap();
    state.settle();
    pixels[32 * 32 * 4 - 4] ^= 1;
    let after = prepare_snapshot(make(pixels), &[], &mut state, &mut rejected, now).unwrap();
    assert_ne!(before.hash, after.hash);
}

#[test]
fn old_tmux_content_cannot_suppress_independent_clipboard_recopies() {
    let text = "previous tmux value";
    let hash = compute_hash(text.as_bytes());
    let db = StorageEngine::new_in_memory().unwrap();
    let old = db
        .insert_clip(
            &ContentType::Text,
            Some(text),
            None,
            None,
            &hash,
            text.len() as i64,
            false,
        )
        .unwrap();
    db.delete_clip(old.id).unwrap();
    let mut state = PollState::default();
    let mut rejected = None;
    // tmux 自身的最后文件哈希已不进入系统快照处理；即便旧条目已删除，新的
    // selection 也必须重新入库。内部主动复制仍由当前 WriteEpoch 哈希抑制。
    for _ in 0..2 {
        state.reset();
        let item = prepare_snapshot(
            ClipboardSnapshot::Text(text.into()),
            &[],
            &mut state,
            &mut rejected,
            Instant::now(),
        )
        .unwrap();
        let clip = db
            .insert_clip(
                &item.kind,
                item.text.as_deref(),
                None,
                None,
                &item.hash,
                item.byte_size,
                item.sensitive,
            )
            .unwrap();
        assert_ne!(clip.id, old.id);
        state.settle();
    }
    assert_eq!(db.get_clips(None, false, 0, 10).unwrap().len(), 1);
    state.reset();
    assert!(prepare_snapshot(
        ClipboardSnapshot::Text(text.into()),
        &[hash],
        &mut state,
        &mut rejected,
        Instant::now()
    )
    .is_none());
}
