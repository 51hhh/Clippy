use super::{
    enhanced::{cached, remember},
    native_layout::group_lines,
    runtime::{ocr_runtime, prepare_permit},
    tesseract::RECOGNITION_TIMEOUT,
    OcrLine, OcrParagraph, OcrPipeline, StructuredOcr, StructuredResult,
};
use clippy_ocr_protocol::{
    rgba_bytes, validate_lines, Line, Reply, Request, ENGINE_ID, MAX_REPLY_BYTES, WORKER_NAME,
};
use image::RgbaImage;
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot},
};

const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const CANCELLATION_POLL: Duration = Duration::from_millis(20);

pub struct NativeOcr {
    bundle: PathBuf,
    instance: u64,
    state: Mutex<State>,
}

enum State {
    Idle,
    Running(Supervisor),
    Stopped,
}

struct Supervisor {
    requests: mpsc::Sender<Job>,
    stop: oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
}

struct Job {
    key: String,
    image: Arc<RgbaImage>,
    complete: oneshot::Sender<StructuredResult>,
}

impl NativeOcr {
    pub fn new(bundle: PathBuf) -> Self {
        static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(0);
        Self {
            bundle,
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            state: Mutex::new(State::Idle),
        }
    }

    pub async fn recognize(
        self: Arc<Self>,
        image: Arc<RgbaImage>,
        cancelled: Arc<AtomicBool>,
    ) -> StructuredResult {
        rgba_bytes(image.width(), image.height())?;
        if cancelled.load(Ordering::Acquire) {
            return Err("OCR 请求已取消".into());
        }
        let preparation = prepare_permit()?;
        let input = Arc::clone(&image);
        let instance = self.instance;
        let key = tokio::task::spawn_blocking(move || {
            let _preparation = preparation;
            format!(
                "{ENGINE_ID}:{instance}:{}x{}:{:x}",
                input.width(),
                input.height(),
                Sha256::digest(input.as_raw()),
            )
        })
        .await
        .map_err(|_| "OCR 图像身份计算失败".to_string())?;
        ocr_runtime()
            .run_structured_cancellable(key.clone(), Some(cancelled), move || async move {
                if let Some(result) = cached(&key) {
                    return Ok(result);
                }
                let (complete, result) = oneshot::channel();
                self.sender()?
                    .send(Job {
                        key: key.clone(),
                        image,
                        complete,
                    })
                    .await
                    .map_err(|_| "OCR 监督线程已退出".to_string())?;
                let result = result
                    .await
                    .map_err(|_| "OCR 监督线程已退出".to_string())??;
                remember(key, &result);
                Ok(result)
            })
            .await
    }

    fn sender(&self) -> Result<mpsc::Sender<Job>, String> {
        let mut state = self.state.lock().expect("OCR 监督状态锁被污染");
        match &*state {
            State::Running(supervisor) => return Ok(supervisor.requests.clone()),
            State::Stopped => return Err("OCR 监督线程已关闭".into()),
            State::Idle => {}
        }
        if !self.bundle.is_absolute() {
            return Err("OCR bundle directory must be absolute".into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("启动 OCR 监督运行时失败: {error}"))?;
        let (requests, receiver) = mpsc::channel(1);
        let (stop, stopped) = oneshot::channel();
        let bundle = self.bundle.clone();
        let thread = thread::Builder::new()
            .name("clippy-ocr-supervisor".into())
            .spawn(move || runtime.block_on(supervise(&bundle, receiver, stopped)))
            .map_err(|error| format!("启动 OCR 监督线程失败: {error}"))?;
        *state = State::Running(Supervisor {
            requests: requests.clone(),
            stop,
            thread,
        });
        Ok(requests)
    }

    pub fn shutdown(&self) {
        let state = std::mem::replace(
            &mut *self.state.lock().expect("OCR 监督状态锁被污染"),
            State::Stopped,
        );
        if let State::Running(supervisor) = state {
            let _ = supervisor.stop.send(());
            let _ = supervisor.thread.join();
        }
    }
}

impl Drop for NativeOcr {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn supervise(
    bundle: &Path,
    mut requests: mpsc::Receiver<Job>,
    mut stopped: oneshot::Receiver<()>,
) {
    let mut worker = None;
    let mut id = 0;
    loop {
        let job = tokio::select! {
            biased;
            _ = &mut stopped => break,
            job = requests.recv() => match job {
                Some(job) => job,
                None => break,
            },
            _ = tokio::time::sleep(IDLE_TIMEOUT), if worker.is_some() => {
                reap(&mut worker).await;
                continue;
            }
        };
        id += 1;
        let result = tokio::select! {
            biased;
            _ = &mut stopped => {
                reap(&mut worker).await;
                let _ = job.complete.send(Err("OCR 监督线程已关闭".into()));
                return;
            }
            _ = no_consumers(&job.key) => Err("OCR 请求已取消".into()),
            result = tokio::time::timeout(
                RECOGNITION_TIMEOUT,
                recognize_in_worker(bundle, &mut worker, id, &job.image),
            ) => result.unwrap_or_else(|_| Err("OCR 识别超时".into())),
        };
        if result.is_err() {
            reap(&mut worker).await;
        }
        let _ = job.complete.send(result);
    }
    reap(&mut worker).await;
}

async fn no_consumers(key: &str) {
    while !ocr_runtime().retire_abandoned_snapshot(key) {
        tokio::time::sleep(CANCELLATION_POLL).await;
    }
}

async fn reap(worker: &mut Option<Session>) {
    if let Some(mut worker) = worker.take() {
        // 回收完成后才通知共享调度器，让排队任务取得执行许可。
        let _ = worker.child.kill().await;
        let _ = worker.child.wait().await;
    }
}

async fn recognize_in_worker(
    bundle: &Path,
    worker: &mut Option<Session>,
    id: u64,
    image: &RgbaImage,
) -> StructuredResult {
    if worker.is_none() {
        *worker = Some(Session::spawn(bundle)?);
    }
    let request = Request {
        id,
        width: image.width(),
        height: image.height(),
    };
    let lines = worker
        .as_mut()
        .expect("OCR worker 已启动")
        .recognize(request, image.as_raw())
        .await?;
    Ok(structured(request, lines))
}

struct Session {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
}

impl Session {
    fn spawn(bundle: &Path) -> Result<Self, String> {
        let mut command = Command::new(bundle.join(WORKER_NAME));
        command
            .arg(bundle)
            .env("ORT_DISABLE_TELEMETRY", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Cannot start offline OCR worker: {error}"))?;
        let input = child.stdin.take().expect("OCR stdin 已配置");
        let output = child.stdout.take().expect("OCR stdout 已配置");
        Ok(Self {
            child,
            input,
            output,
        })
    }

    async fn recognize(&mut self, request: Request, pixels: &[u8]) -> Result<Vec<Line>, String> {
        let (_, reply) = tokio::try_join!(
            async {
                self.input.write_all(&request.encode()).await?;
                self.input.write_all(pixels).await?;
                self.input.flush().await
            },
            read_reply(&mut self.output),
        )
        .map_err(|error| format!("Offline OCR worker communication failed: {error}"))?;
        if reply.id != request.id {
            return Err("Offline OCR reply did not match its request".into());
        }
        let lines = reply.result?;
        validate_lines(&lines, request)?;
        Ok(lines)
    }
}

async fn read_reply(output: &mut ChildStdout) -> io::Result<Reply> {
    let bytes = output.read_u32_le().await? as usize;
    if bytes == 0 || bytes > MAX_REPLY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OCR reply exceeds the pipe budget",
        ));
    }
    let mut reply = vec![0; bytes];
    output.read_exact(&mut reply).await?;
    serde_json::from_slice(&reply)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn structured(request: Request, source: Vec<Line>) -> StructuredOcr {
    let groups = group_lines(&source);
    let count = source.len();
    let mut source: Vec<_> = source.into_iter().map(Some).collect();
    let mut text = String::new();
    let mut lines = Vec::with_capacity(count);
    let mut paragraphs = Vec::with_capacity(groups.len());
    for (index, group) in groups.into_iter().enumerate() {
        let paragraph_id = index as u32;
        if index > 0 {
            text.push_str("\n\n");
        }
        let mut line_ids = Vec::with_capacity(group.len());
        for (offset, index) in group.into_iter().enumerate() {
            let line = source[index].take().expect("排版为每行分配唯一段落");
            if offset > 0 {
                text.push('\n');
            }
            text.push_str(&line.text);
            let id = lines.len() as u32;
            line_ids.push(id);
            lines.push(OcrLine {
                id,
                quad: line.quad.to_vec(),
                text: line.text,
                accepted: true,
                confidence: line.confidence,
                char_confidences: Vec::new(),
                paragraph_id,
                reading_order: id,
            });
        }
        paragraphs.push(OcrParagraph {
            id: paragraph_id,
            line_ids,
            reading_order: paragraph_id,
        });
    }
    StructuredOcr {
        width: request.width,
        height: request.height,
        text,
        lines,
        paragraphs,
        pipeline: OcrPipeline {
            id: ENGINE_ID.into(),
            engine: ENGINE_ID.into(),
            feature_schema: None,
            layout_executed: count > 1,
            layout_reason: match count {
                0 => Some("no_text".into()),
                1 => Some("single_line".into()),
                _ => None,
            },
        },
        fallback_reason: None,
    }
}
