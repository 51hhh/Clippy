//! Clippy 的隔离 X11 协议回归；默认忽略，必须在独立 Xvfb 显式运行。
#![cfg(target_os = "linux")]

use std::io::{BufRead, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn pixels(width: usize, height: usize, seed: u32) -> Vec<u8> {
    let mut bytes = vec![0; width * height * 4];
    let mut value = seed;
    for pixel in bytes.as_chunks_mut::<4>().0 {
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        pixel.copy_from_slice(&[value as u8, (value >> 8) as u8, (value >> 16) as u8, 255]);
    }
    bytes
}

fn provider() -> bool {
    assert_eq!(
        std::env::var("CLIPPY_TEST_X11_ISOLATED").as_deref(),
        Ok("1")
    );
    if std::env::var_os("CLIPPY_INCR_PROVIDER").is_none() {
        return false;
    }
    let mut clipboard = arboard::Clipboard::new().unwrap();
    for command in std::io::stdin().lock().lines() {
        let command = command.unwrap();
        if command == "drop" {
            drop(clipboard);
            println!("CLIPPY_READY");
            std::io::stdout().flush().unwrap();
            return true;
        } else if let Some(text) = command.strip_prefix("text ") {
            clipboard.set_text(text).unwrap();
        } else {
            let args: Vec<usize> = command
                .split_whitespace()
                .map(|s| s.parse().unwrap())
                .collect();
            clipboard
                .set_image(arboard::ImageData {
                    width: args[0],
                    height: args[1],
                    bytes: pixels(args[0], args[1], args[2] as u32).into(),
                })
                .unwrap();
        }
        println!("CLIPPY_READY");
        std::io::stdout().flush().unwrap();
    }
    true
}

struct Provider {
    child: Child,
    ready: Receiver<()>,
}
impl Provider {
    fn new(test: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--ignored", "--nocapture"])
            .env("CLIPPY_INCR_PROVIDER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, ready) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if line == "CLIPPY_READY" {
                    let _ = send.send(());
                }
            }
        });
        Self { child, ready }
    }
    fn send(&mut self, command: &str) {
        let input = self.child.stdin.as_mut().unwrap();
        writeln!(input, "{command}").unwrap();
        input.flush().unwrap();
    }
    fn write(&mut self, command: &str) {
        self.send(command);
        self.ready.recv_timeout(Duration::from_secs(20)).unwrap();
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn xclip_image() -> Result<Vec<u8>, String> {
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-target", "image/png", "-out"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().unwrap();
    let reading = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    let success = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status.success();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = reading.join().unwrap();
    if success {
        Ok(bytes)
    } else {
        Err("xclip failed or exceeded 8s".into())
    }
}

#[test]
#[ignore = "requires private Xvfb and CLIPPY_TEST_X11_ISOLATED=1"]
fn private_x11_large_image_roundtrip() {
    if provider() {
        return;
    }
    let mut provider = Provider::new("private_x11_large_image_roundtrip");
    let mut clipboard = arboard::Clipboard::new().unwrap();
    let mut errors = Vec::new();
    for (width, height) in [(3840, 2160), (7680, 4320)] {
        provider.write(&format!("{width} {height} 2268358059"));
        let expected = pixels(width, height, 2268358059);
        let started = Instant::now();
        match clipboard.get_image() {
            Ok(image) => {
                assert_eq!((image.width, image.height), (width, height));
                assert_eq!(image.bytes.as_ref(), expected);
                println!(
                    "arboard {width}x{height} pixels equal in {:?}",
                    started.elapsed()
                );
            }
            Err(error) => {
                println!("arboard {width}x{height} failed: {error}");
                errors.push(error.to_string());
            }
        }
        let started = Instant::now();
        match xclip_image() {
            Ok(bytes) => {
                let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                    .unwrap()
                    .into_rgba8();
                assert_eq!(decoded.as_raw(), &expected);
                println!(
                    "xclip {width}x{height} {} bytes, pixels equal in {:?}",
                    bytes.len(),
                    started.elapsed()
                );
            }
            Err(error) => {
                println!("xclip {width}x{height} failed: {error}");
                errors.push(error);
            }
        }
    }
    provider.write("text after large images");
    assert_eq!(clipboard.get_text().unwrap(), "after large images");
    assert!(errors.is_empty(), "{errors:?}");
}

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, Property, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

struct RawReader {
    connection: RustConnection,
    window: u32,
    property: u32,
    target: u32,
    selection: u32,
}
impl RawReader {
    fn new() -> Self {
        let (connection, screen) = x11rb::connect(None).unwrap();
        let window = connection.generate_id().unwrap();
        connection
            .create_window(
                0,
                window,
                connection.setup().roots[screen].root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .unwrap()
            .check()
            .unwrap();
        let property = connection
            .intern_atom(false, b"CLIPPY_INCR_TEST")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let target = connection
            .intern_atom(false, b"image/png")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let selection = connection
            .intern_atom(false, b"CLIPBOARD")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        Self {
            connection,
            window,
            property,
            target,
            selection,
        }
    }
    fn next_event(&self, deadline: Instant) -> Event {
        loop {
            if let Some(event) = self.connection.poll_for_event().unwrap() {
                return event;
            }
            assert!(Instant::now() < deadline, "requestor event deadline");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn request(&self) -> bool {
        self.connection
            .delete_property(self.window, self.property)
            .unwrap()
            .check()
            .unwrap();
        self.connection
            .convert_selection(
                self.window,
                self.selection,
                self.target,
                self.property,
                x11rb::CURRENT_TIME,
            )
            .unwrap()
            .check()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Event::SelectionNotify(event) = self.next_event(deadline) {
                if event.property == 0 {
                    return false;
                }
                let response = self
                    .connection
                    .get_property(
                        false,
                        self.window,
                        self.property,
                        AtomEnum::ANY,
                        0,
                        u32::MAX,
                    )
                    .unwrap()
                    .reply()
                    .unwrap();
                let incr = self
                    .connection
                    .intern_atom(false, b"INCR")
                    .unwrap()
                    .reply()
                    .unwrap()
                    .atom;
                assert_eq!(response.type_, incr, "此场景必须走生产 INCR 阈值");
                assert_eq!(response.format, 32);
                assert_eq!(response.value_len, 1);
                return true;
            }
        }
    }
    fn conflicting_small_request_is_refused(&self) {
        let text = self
            .connection
            .intern_atom(false, b"UTF8_STRING")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        self.connection
            .convert_selection(
                self.window,
                self.selection,
                text,
                self.property,
                x11rb::CURRENT_TIME,
            )
            .unwrap()
            .check()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Event::SelectionNotify(event) = self.next_event(deadline) {
                assert_eq!(event.target, text);
                assert_eq!(event.property, 0, "冲突的小文本不能覆盖既有 INCR");
                break;
            }
        }
    }
    fn finish(&self, delay_first: bool) -> Vec<u8> {
        self.read_chunks(delay_first, true)
    }
    fn read_chunks(&self, delay_first: bool, acknowledge_final: bool) -> Vec<u8> {
        self.read_chunks_paced(delay_first, acknowledge_final, Duration::ZERO)
    }
    fn read_chunks_paced(
        &self,
        delay_first: bool,
        acknowledge_final: bool,
        delay: Duration,
    ) -> Vec<u8> {
        // 初始 INCR 属性先保持不删，模拟不消费的接收者，之后才确认。
        if delay_first {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.connection
            .delete_property(self.window, self.property)
            .unwrap()
            .check()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut bytes = Vec::new();
        let mut first = true;
        loop {
            if let Event::PropertyNotify(event) = self.next_event(deadline) {
                if event.atom != self.property || event.state != Property::NEW_VALUE {
                    continue;
                }
                if delay_first && first {
                    std::thread::sleep(Duration::from_millis(50));
                }
                first = false;
                std::thread::sleep(delay);
                let reply = self
                    .connection
                    .get_property(false, self.window, self.property, self.target, 0, u32::MAX)
                    .unwrap()
                    .reply()
                    .unwrap();
                assert_eq!(reply.type_, self.target);
                assert_eq!(reply.format, 8);
                assert_eq!(reply.bytes_after, 0);
                if !reply.value.is_empty() || acknowledge_final {
                    self.connection
                        .delete_property(self.window, self.property)
                        .unwrap()
                        .check()
                        .unwrap();
                }
                if reply.value.is_empty() {
                    return bytes;
                }
                bytes.extend(reply.value);
            }
        }
    }
    fn hold_first_chunk(&self) {
        self.connection
            .delete_property(self.window, self.property)
            .unwrap()
            .check()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Event::PropertyNotify(event) = self.next_event(deadline) {
                if event.atom == self.property && event.state == Property::NEW_VALUE {
                    let reply = self
                        .connection
                        .get_property(false, self.window, self.property, self.target, 0, u32::MAX)
                        .unwrap()
                        .reply()
                        .unwrap();
                    assert!(!reply.value.is_empty());
                    return; // 故意不确认第一块。
                }
            }
        }
    }
}
impl Drop for RawReader {
    fn drop(&mut self) {
        let _ = self.connection.destroy_window(self.window);
        let _ = self.connection.flush();
    }
}

fn assert_png(bytes: &[u8], expected: &[u8]) {
    assert_eq!(
        image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8()
            .as_raw(),
        expected
    );
}

#[test]
#[ignore = "requires private Xvfb and CLIPPY_TEST_X11_ISOLATED=1"]
fn private_x11_concurrent_snapshots_and_destroyed_or_stalled_requestors() {
    if provider() {
        return;
    }
    let mut provider =
        Provider::new("private_x11_concurrent_snapshots_and_destroyed_or_stalled_requestors");
    provider.write("3840 2160 2268358059");
    let expected = pixels(3840, 2160, 2268358059);
    let first = RawReader::new();
    let second = RawReader::new();
    assert!(first.request());
    assert!(second.request());
    // 两个相同源 reader 共用 payload；同 owner 新复制不修改已接受的老快照。
    provider.write("text replaced by same owner");
    first.conflicting_small_request_is_refused();
    assert_png(&second.finish(true), &expected);
    // 外部 owner 抢 selection 触发 SelectionClear，已有传输仍必须完整结束。
    let mut external = arboard::Clipboard::new().unwrap();
    external.set_text("external owner").unwrap();
    assert_png(&first.finish(false), &expected);
    assert_eq!(external.get_text().unwrap(), "external owner");
    drop(first);
    drop(second);

    provider.write("3840 2160 2268358059");
    let mut blocked: Vec<_> = (0..4).map(|_| RawReader::new()).collect();
    for reader in &blocked {
        assert!(reader.request());
    }
    blocked[1].hold_first_chunk();
    assert_png(&blocked[2].read_chunks(false, false), &expected); // 故意不确认终止零块。
    let extra = RawReader::new();
    assert!(!extra.request(), "并发第 5 个请求应明确拒绝，不排无限队列");
    drop(blocked.pop()); // 接收窗口销毁只释放自己的 transfer，不能停止服务。
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if extra.request() {
            break;
        }
        assert!(Instant::now() < deadline, "destroy did not release slot");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_png(&extra.finish(false), &expected);
    // 其余 3 个 receiver 分别挂在初始、数据、最终 ack；清理后应重新容纳 4 个请求。
    let started = Instant::now();
    loop {
        let probes: Vec<_> = (0..4).map(|_| RawReader::new()).collect();
        let accepted = probes.iter().filter(|reader| reader.request()).count();
        if accepted == 4 {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "stalled requestors retained resources"
        );
        drop(probes);
        std::thread::sleep(Duration::from_millis(50));
    }
    println!(
        "stalled receiver slots reclaimed in {:?}",
        started.elapsed()
    );
    drop(blocked);
    drop(extra);
    provider.write("text server still running after destroyed and expired readers");
    assert_eq!(
        external.get_text().unwrap(),
        "server still running after destroyed and expired readers"
    );
}

#[test]
#[ignore = "requires private Xvfb and CLIPPY_TEST_X11_ISOLATED=1"]
fn private_x11_handover_waits_for_data_confirmation_and_remains_bounded() {
    use x11rb::protocol::xproto::{SelectionNotifyEvent, SELECTION_NOTIFY_EVENT};
    if provider() {
        return;
    }
    // 没有 manager 不应平白等完整的 handover 截止。
    let mut plain =
        Provider::new("private_x11_handover_waits_for_data_confirmation_and_remains_bounded");
    plain.write("text no manager");
    let started = Instant::now();
    plain.write("drop");
    assert!(started.elapsed() < Duration::from_secs(1));

    let manager = RawReader::new();
    let selection = manager
        .connection
        .intern_atom(false, b"CLIPBOARD_MANAGER")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    manager
        .connection
        .set_selection_owner(manager.window, selection, x11rb::CURRENT_TIME)
        .unwrap()
        .check()
        .unwrap();
    for consume in [true, false] {
        let mut owner =
            Provider::new("private_x11_handover_waits_for_data_confirmation_and_remains_bounded");
        owner.write("7680 4320 2268358059");
        owner.send("drop");
        let deadline = Instant::now() + Duration::from_secs(5);
        let request = loop {
            if let Event::SelectionRequest(event) = manager.next_event(deadline) {
                break event;
            }
        };
        assert_eq!(request.selection, selection);
        // 模拟先发送 SAVE_TARGETS 成功通知、后请求大 PNG 的真实 manager 交错。
        manager
            .connection
            .send_event(
                false,
                request.requestor,
                EventMask::NO_EVENT,
                SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property: request.property,
                },
            )
            .unwrap()
            .check()
            .unwrap();
        assert!(manager.request());
        assert!(
            owner
                .ready
                .recv_timeout(Duration::from_millis(200))
                .is_err(),
            "INCR header 不是 handover 完成"
        );
        let started = Instant::now();
        if consume {
            let bytes = manager.read_chunks_paced(false, false, Duration::from_millis(50));
            assert_png(&bytes, &pixels(7680, 4320, 2268358059));
            assert!(started.elapsed() > Duration::from_secs(4));
            assert!(
                owner
                    .ready
                    .recv_timeout(Duration::from_millis(100))
                    .is_err(),
                "最终零块尚未确认"
            );
            manager
                .connection
                .delete_property(manager.window, manager.property)
                .unwrap()
                .check()
                .unwrap();
            owner.ready.recv_timeout(Duration::from_secs(2)).unwrap();
        } else {
            // manager 永不删除初始属性，Drop 仍应在有界时间内返回并回收 sender。
            owner.ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let remaining = manager
                .connection
                .get_property(
                    false,
                    manager.window,
                    manager.property,
                    AtomEnum::ANY,
                    0,
                    u32::MAX,
                )
                .unwrap()
                .reply()
                .unwrap();
            assert_eq!(remaining.type_, 0, "超时退出应清理遗留 INCR 属性");
        }
        println!(
            "manager consume={consume} exited after {:?}",
            started.elapsed()
        );
    }
}

#[test]
#[ignore = "requires private Xvfb and CLIPPY_TEST_X11_ISOLATED=1"]
fn private_x11_arboard_accepts_legal_slow_incr_chunks() {
    use image::ImageEncoder;
    use x11rb::protocol::xproto::{
        ChangeWindowAttributesAux, PropMode, SelectionNotifyEvent, SELECTION_NOTIFY_EVENT,
    };
    use x11rb::wrapper::ConnectionExt as _;
    assert_eq!(
        std::env::var("CLIPPY_TEST_X11_ISOLATED").as_deref(),
        Ok("1")
    );
    let owner = RawReader::new();
    owner
        .connection
        .set_selection_owner(owner.window, owner.selection, x11rb::CURRENT_TIME)
        .unwrap()
        .check()
        .unwrap();
    let mut clipboard = arboard::Clipboard::new().unwrap();
    let reading = std::thread::spawn(move || clipboard.get_image());
    let deadline = Instant::now() + Duration::from_secs(5);
    let request = loop {
        if let Event::SelectionRequest(event) = owner.next_event(deadline) {
            break event;
        }
    };
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[9, 22, 83, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    let incr = owner
        .connection
        .intern_atom(false, b"INCR")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    owner
        .connection
        .change_window_attributes(
            request.requestor,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .unwrap()
        .check()
        .unwrap();
    owner
        .connection
        .change_property32(
            PropMode::REPLACE,
            request.requestor,
            request.property,
            incr,
            &[png.len() as u32],
        )
        .unwrap()
        .check()
        .unwrap();
    owner
        .connection
        .send_event(
            false,
            request.requestor,
            EventMask::NO_EVENT,
            SelectionNotifyEvent {
                response_type: SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: request.time,
                requestor: request.requestor,
                selection: request.selection,
                target: request.target,
                property: request.property,
            },
        )
        .unwrap()
        .check()
        .unwrap();
    for chunk in png.chunks(30).chain(std::iter::once(&[][..])) {
        loop {
            if let Event::PropertyNotify(event) = owner.next_event(deadline) {
                if event.window == request.requestor
                    && event.atom == request.property
                    && event.state == Property::DELETE
                {
                    break;
                }
            }
        }
        // 合法 provider 受调度影响；旧 10ms 截止会在第二块之前错误退出。
        std::thread::sleep(Duration::from_millis(50));
        owner
            .connection
            .change_property8(
                PropMode::APPEND,
                request.requestor,
                request.property,
                request.target,
                chunk,
            )
            .unwrap()
            .check()
            .unwrap();
    }
    let received = reading.join().unwrap().unwrap();
    assert_eq!((received.width, received.height), (1, 1));
    assert_eq!(received.bytes.as_ref(), &[9, 22, 83, 255]);
}
