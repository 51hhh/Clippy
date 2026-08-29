//! provider 测试共用的回环 HTTP mock。
//!
//! 每个 provider 都要有一条“真的发出请求、真的解析响应”的回环测试，否则请求体拼装
//! 只能靠单元测试断言 JSON 结构，头部、方法和路径都无人覆盖。真实第三方端点不能进测试。

use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

/// mock 服务器要返回的一次响应。
pub(super) struct MockResponse {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl MockResponse {
    pub(super) fn json(body: Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: body.to_string(),
        }
    }

    pub(super) fn text(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
        }
    }

    pub(super) fn html(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: body.into(),
        }
    }

    pub(super) fn audio(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "audio/mpeg",
            body: body.into(),
        }
    }

    pub(super) fn status(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.into(),
        }
    }
}

/// 捕获到的请求。head 保留原始报文头，便于断言方法、路径和自定义头。
pub(super) struct CapturedRequest {
    pub(super) head: String,
    pub(super) body: String,
}

impl CapturedRequest {
    /// 请求行的 request-target（含查询串）。
    pub(super) fn target(&self) -> &str {
        self.head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
    }

    pub(super) fn method(&self) -> &str {
        self.head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().next())
            .unwrap_or_default()
    }

    pub(super) fn header(&self, name: &str) -> Option<String> {
        self.head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim().to_string())
    }

    pub(super) fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("请求体不是合法 JSON")
    }

    /// 解析 `application/x-www-form-urlencoded` 请求体。
    pub(super) fn form(&self) -> HashMap<String, String> {
        url::form_urlencoded::parse(self.body.as_bytes())
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect()
    }

    /// 解析查询串参数。
    pub(super) fn query(&self) -> HashMap<String, String> {
        let query = self.target().split_once('?').map(|(_, q)| q).unwrap_or("");
        url::form_urlencoded::parse(query.as_bytes())
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect()
    }
}

/// 回环 mock 服务器。按顺序为每个连接返回一条预设响应。
pub(super) struct MockServer {
    pub(super) base_url: String,
    receiver: Receiver<CapturedRequest>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    pub(super) fn new(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let captured = read_request(&mut stream);
                sender.send(captured).unwrap();
                write_response(&mut stream, &response);
            }
        });
        Self {
            base_url: format!("http://{address}"),
            receiver,
            handle: Some(handle),
        }
    }

    pub(super) fn json_once(response: Value) -> Self {
        Self::new(vec![MockResponse::json(response)])
    }

    pub(super) fn recv(&self) -> CapturedRequest {
        self.receiver.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    /// 等待服务线程收尾，把服务端的断言失败也变成测试失败。
    pub(super) fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> CapturedRequest {
    let mut request = Vec::new();
    let (header_end, content_length) = loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "HTTP 请求在 header 完成前关闭");
        request.extend_from_slice(&chunk[..count]);
        if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&request[..header_end]);
            let length = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if request.len() >= header_end + 4 + length {
                break (header_end, length);
            }
        }
    };

    let head = String::from_utf8(request[..header_end].to_vec()).unwrap();
    let body_start = header_end + 4;
    let body =
        String::from_utf8(request[body_start..body_start + content_length].to_vec()).unwrap();
    CapturedRequest { head, body }
}

fn write_response(stream: &mut std::net::TcpStream, response: &MockResponse) {
    let wire = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        response.content_type,
        response.body.len(),
        response.body
    );
    stream.write_all(wire.as_bytes()).unwrap();
}
