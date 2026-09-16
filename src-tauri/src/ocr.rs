//! OCR 调度：增强管线与 Tesseract 共用队列、预算和进程回收。
//! 通过命令行调用系统 tesseract，避免编译时动态链接依赖。
//! tesseract 缺失时返回友好错误，不影响应用启动。

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Semaphore};

const TESSERACT_PATH_ENV: &str = "CLIPPY_TESSERACT_PATH";
const OCR_MAX_CONCURRENCY: usize = 1;
const OCR_MAX_QUEUED: usize = 2;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const RECOGNITION_TIMEOUT: Duration = Duration::from_secs(60);
const OCR_STDOUT_LIMIT: usize = 4 * 1024 * 1024;
const OCR_STDERR_LIMIT: usize = 64 * 1024;
mod enhanced;
mod protocol;
pub use protocol::StructuredOcr;
type OcrResult = Result<String, String>;
type StructuredResult = Result<StructuredOcr, String>;

struct OcrRuntime {
    permits: Arc<Semaphore>,
    admission: Arc<Semaphore>,
    #[cfg(test)]
    in_flight: Mutex<HashMap<String, Vec<oneshot::Sender<OcrResult>>>>,
    snapshots: Mutex<HashMap<String, Vec<oneshot::Sender<StructuredResult>>>>,
}

impl OcrRuntime {
    fn new(max_concurrency: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrency)),
            admission: Arc::new(Semaphore::new(max_concurrency + OCR_MAX_QUEUED)),
            #[cfg(test)]
            in_flight: Mutex::new(HashMap::new()),
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    async fn run_image<F, Fut>(&'static self, work: F) -> OcrResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = OcrResult> + Send + 'static,
    {
        let admission = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| "OCR 队列已满，请稍后重试".to_string())?;
        let (mut sender, receiver) = oneshot::channel();
        tauri::async_runtime::spawn(async move {
            let _admission = admission;
            // 仍在排队且唯一消费者已离开，立即释放其冻结 PNG。
            let permit = tokio::select! {
                permit = self.permits.acquire() => permit,
                _ = sender.closed() => return,
            };
            let result = match permit {
                Ok(_permit) => work().await,
                Err(_) => Err("OCR 并发控制器已关闭".to_string()),
            };
            let _ = sender.send(result);
        });
        receiver
            .await
            .unwrap_or_else(|_| Err("OCR 任务意外结束".to_string()))
    }

    #[cfg(test)]
    async fn run_clip<F, Fut>(&'static self, id: impl ToString, work: F) -> OcrResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = OcrResult> + Send + 'static,
    {
        let id = id.to_string();
        let (sender, receiver) = oneshot::channel();
        let should_start = {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match in_flight.get_mut(&id) {
                Some(waiters) => {
                    if waiters.len() >= 64 {
                        return Err("OCR 等待者超过上限".into());
                    }
                    waiters.push(sender);
                    None
                }
                None => {
                    // 在接纳新任务前限制队列；同 ID 合并不重复占位。
                    let admission = self
                        .admission
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| "OCR 队列已满，请稍后重试".to_string())?;
                    in_flight.insert(id.clone(), vec![sender]);
                    Some(admission)
                }
            }
        };
        if let Some(admission) = should_start {
            tauri::async_runtime::spawn(async move {
                let _admission = admission;
                let result = async {
                    let _permit = self
                        .permits
                        .acquire()
                        .await
                        .map_err(|_| "OCR 并发控制器已关闭".to_string())?;
                    let has_waiters = {
                        let mut flights = self
                            .in_flight
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        let waiters = flights.get_mut(&id).expect("任务持有已登记的 OCR 身份");
                        waiters.retain(|waiter| !waiter.is_closed());
                        !waiters.is_empty()
                    };
                    if !has_waiters {
                        return Err("OCR 排队请求已取消".to_string());
                    }
                    work().await
                }
                .await;
                let waiters = self
                    .in_flight
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&id)
                    .unwrap_or_default();
                for waiter in waiters {
                    let _ = waiter.send(result.clone());
                }
            });
        } else {
            drop(work);
        }
        receiver
            .await
            .unwrap_or_else(|_| Err("OCR 任务意外结束".to_string()))
    }
}

fn ocr_runtime() -> &'static OcrRuntime {
    static RUNTIME: OnceLock<OcrRuntime> = OnceLock::new();
    RUNTIME.get_or_init(|| OcrRuntime::new(OCR_MAX_CONCURRENCY))
}

impl OcrRuntime {
    fn has_consumers(&self, key: &str) -> bool {
        self.snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .is_some_and(|waiters| waiters.iter().any(|waiter| !waiter.is_closed()))
    }

    async fn run_structured<F, Fut>(&'static self, key: String, work: F) -> StructuredResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = StructuredResult> + Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let admission = {
            let mut flights = self
                .snapshots
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(waiters) = flights.get_mut(&key) {
                waiters.retain(|waiter| !waiter.is_closed());
                if waiters.len() >= 64 {
                    return Err("OCR 等待者超过上限".into());
                }
                waiters.push(sender);
                None
            } else {
                let admission = self
                    .admission
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| "OCR 队列已满，请稍后重试".to_string())?;
                flights.insert(key.clone(), vec![sender]);
                Some(admission)
            }
        };
        if let Some(admission) = admission {
            tauri::async_runtime::spawn(async move {
                let _admission = admission;
                let result = async {
                    let _permit = self
                        .permits
                        .acquire()
                        .await
                        .map_err(|_| "OCR 并发控制器已关闭".to_string())?;
                    let waiting = self
                        .snapshots
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .get(&key)
                        .is_some_and(|waiters| waiters.iter().any(|waiter| !waiter.is_closed()));
                    if !waiting {
                        return Err("OCR 排队请求已取消".into());
                    }
                    work().await
                }
                .await;
                let waiters = self
                    .snapshots
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&key)
                    .unwrap_or_default();
                for waiter in waiters {
                    let _ = waiter.send(result.clone());
                }
            });
        } else {
            // 合并消费者不持有它自己那一份大 PNG 到识别结束。
            drop(work);
        }
        receiver
            .await
            .unwrap_or_else(|_| Err("OCR 任务意外结束".into()))
    }
}

/// 查看器等不可变图像快照的结构化 OCR；与旧字符串入口共用同一全局预算。
pub async fn recognize_snapshot(png: Vec<u8>) -> StructuredResult {
    recognize_snapshot_shared(Arc::new(png)).await
}

/// 重复查看器请求共享冻结 PNG；排队/配置阶段不复制整张图片。
pub async fn recognize_snapshot_shared(png: Arc<Vec<u8>>) -> StructuredResult {
    enhanced::image_dimensions(&png)?;
    let preparation = prepare_permit()?;
    let input = Arc::clone(&png);
    let (configuration, key) = tauri::async_runtime::spawn_blocking(move || {
        // 调用者取消时 blocking 工作仍持有小准备许可，不产生无限后台配置探测。
        let _preparation = preparation;
        let configuration = enhanced::configuration();
        let key = enhanced::request_key(&input, &configuration);
        (configuration, key)
    })
    .await
    .map_err(|_| "OCR 配置读取失败".to_string())?;
    let runtime = ocr_runtime();
    let cancellation_key = key.clone();
    runtime
        .run_structured(key, move || {
            enhanced::recognize_owned(png, configuration, move || {
                !runtime.has_consumers(&cancellation_key)
            })
        })
        .await
}

// 文件校验与哈希也只有三个准备任务，不在异步执行器上扫描整幅 PNG。
fn prepare_permit() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static PREPARE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PREPARE
        .get_or_init(|| Arc::new(Semaphore::new(OCR_MAX_CONCURRENCY + OCR_MAX_QUEUED)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "OCR 队列已满，请稍后重试".to_string())
}

async fn prepare_configuration() -> Result<enhanced::Configuration, String> {
    let permit = prepare_permit()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        enhanced::configuration()
    })
    .await
    .map_err(|_| "OCR 配置读取失败".to_string())
}

/// 旧的无版本 String 缓存无法证明来自当前增强模型，配置增强时必须绕过。
pub(crate) fn uses_enhanced_configuration() -> bool {
    enhanced::configured()
}

fn push_unique(candidates: &mut Vec<PathBuf>, candidate: impl Into<PathBuf>) {
    let candidate = candidate.into();
    if !candidate.as_os_str().is_empty() && !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn bundled_candidate() -> Option<PathBuf> {
    let executable_dir = env::current_exe().ok()?.parent()?.to_path_buf();

    #[cfg(target_os = "windows")]
    return Some(executable_dir.join("tesseract.exe"));

    #[cfg(target_os = "macos")]
    return executable_dir
        .parent()
        .map(|contents| contents.join("Resources").join("tesseract"));

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Some(executable_dir.join("tesseract"))
}

fn platform_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = env::var_os(variable) {
                push_unique(
                    &mut candidates,
                    PathBuf::from(root)
                        .join("Tesseract-OCR")
                        .join("tesseract.exe"),
                );
            }
        }
        if let Some(root) = env::var_os("LOCALAPPDATA") {
            push_unique(
                &mut candidates,
                PathBuf::from(root)
                    .join("Programs")
                    .join("Tesseract-OCR")
                    .join("tesseract.exe"),
            );
        }
        candidates
    }

    #[cfg(target_os = "macos")]
    {
        [
            "/opt/homebrew/bin/tesseract",
            "/usr/local/bin/tesseract",
            "/opt/local/bin/tesseract",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    #[cfg(target_os = "linux")]
    {
        [
            "/usr/bin/tesseract",
            "/usr/local/bin/tesseract",
            "/snap/bin/tesseract",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Vec::new()
}

fn tesseract_candidates(override_path: Option<OsString>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = override_path {
        push_unique(&mut candidates, path);
    }
    if let Some(path) = bundled_candidate() {
        push_unique(&mut candidates, path);
    }
    // 保留 PATH 语义，终端启动和自定义包管理器路径仍可工作。
    push_unique(&mut candidates, "tesseract");
    for path in platform_candidates() {
        push_unique(&mut candidates, path);
    }
    candidates
}

fn first_available<F>(
    candidates: impl IntoIterator<Item = PathBuf>,
    mut probe: F,
) -> Option<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    candidates.into_iter().find(|path| probe(path))
}

fn probe_with_timeout(path: &Path, timeout: Duration) -> bool {
    let mut command = Command::new(path);
    command.arg("--version");
    probe_command_with_timeout(command, timeout)
}

fn probe_command_with_timeout(mut command: Command, timeout: Duration) -> bool {
    let Ok(mut child) = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[derive(Default)]
struct ExecutableCache {
    state: Mutex<ExecutableCacheState>,
    ready: Condvar,
}

#[derive(Default)]
struct ExecutableCacheState {
    generation: u64,
    probing: bool,
    value: Option<Option<PathBuf>>,
}

impl ExecutableCache {
    #[cfg(test)]
    fn resolve_with<F>(&self, resolver: F) -> Option<PathBuf>
    where
        F: FnMut() -> Option<PathBuf>,
    {
        self.resolve_until(Instant::now() + PROBE_TIMEOUT, resolver)
    }

    fn resolve_until<F>(&self, deadline: Instant, mut resolver: F) -> Option<PathBuf>
    where
        F: FnMut() -> Option<PathBuf>,
    {
        loop {
            let generation = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                while state.probing {
                    let remaining = deadline.checked_duration_since(Instant::now())?;
                    let (next, timed) = self
                        .ready
                        .wait_timeout(state, remaining)
                        .unwrap_or_else(|error| error.into_inner());
                    state = next;
                    if timed.timed_out() && state.probing {
                        return None;
                    }
                }
                if let Some(cached) = state.value.as_ref() {
                    return cached.clone();
                }
                if Instant::now() >= deadline {
                    return None;
                }
                state.probing = true;
                state.generation
            };

            // 探测会启动外部进程，不能占着同步锁阻塞其它查询或安装后的失效操作。
            let resolved = resolver();
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.probing = false;
            self.ready.notify_all();
            if state.generation != generation {
                // 探测期间发生过安装/卸载，旧结果不得覆盖新一代缓存。
                continue;
            }
            if let Some(cached) = state.value.as_ref() {
                return cached.clone();
            }
            if resolved.is_some() || Instant::now() < deadline {
                state.value = Some(resolved.clone());
            }
            return resolved;
        }
    }

    fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.value = None;
    }
}

fn executable_cache() -> &'static ExecutableCache {
    static CACHE: OnceLock<ExecutableCache> = OnceLock::new();
    CACHE.get_or_init(ExecutableCache::default)
}

fn tesseract_executable() -> Option<PathBuf> {
    tesseract_executable_until(Instant::now() + PROBE_TIMEOUT)
}

fn tesseract_executable_until(deadline: Instant) -> Option<PathBuf> {
    let deadline = deadline.min(Instant::now() + PROBE_TIMEOUT);
    executable_cache().resolve_until(deadline, || {
        first_available(
            tesseract_candidates(env::var_os(TESSERACT_PATH_ENV)),
            |path| {
                deadline
                    .checked_duration_since(Instant::now())
                    .is_some_and(|remaining| probe_with_timeout(path, remaining))
            },
        )
    })
}

/// 安装流程改变了外部工具状态，下一次查询必须重新探测。
pub(crate) fn invalidate_executable_cache() {
    executable_cache().invalidate();
}

/// 检查系统是否安装了可实际执行的 Tesseract。
pub fn is_available() -> bool {
    enhanced::configuration().is_enhanced() || tesseract_executable().is_some()
}

#[cfg(target_os = "linux")]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未安装 tesseract。请运行 sudo apt install tesseract-ocr tesseract-ocr-chi-sim"
}

#[cfg(target_os = "windows")]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract.exe。请安装 Tesseract 后重启 Clippy，或设置 CLIPPY_TESSERACT_PATH"
}

#[cfg(target_os = "macos")]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract。请通过 Homebrew/MacPorts 安装，或设置 CLIPPY_TESSERACT_PATH"
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract"
}

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| format!("读取 OCR 输出失败: {error}"))?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > limit {
            return Err("OCR 输出超过安全上限".to_string());
        }
        output.extend_from_slice(&chunk[..count]);
    }
}

async fn recognize_with_timeout(
    executable: &Path,
    png_bytes: &[u8],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> OcrResult {
    let mut command = tokio::process::Command::new(executable);
    command.args(["stdin", "stdout", "-l", "eng+chi_sim"]);
    run_recognition_process(command, png_bytes, timeout, stdout_limit, stderr_limit).await
}

async fn run_recognition_process(
    mut command: tokio::process::Command,
    png_bytes: &[u8],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> OcrResult {
    use tokio::io::AsyncWriteExt;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                invalidate_executable_cache();
                missing_tesseract_message().to_string()
            } else {
                format!("启动 tesseract 失败: {error}")
            }
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "OCR stdin 不可用".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "OCR stdout 不可用".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "OCR stderr 不可用".to_string())?;
    // 三条管道并行：输出不能堵住输入；所有 I/O 与退出共同受同一个截止时间约束。
    let result = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            async {
                stdin
                    .write_all(png_bytes)
                    .await
                    .map_err(|error| format!("写入 OCR 图片失败: {error}"))?;
                drop(stdin);
                Ok(())
            },
            read_bounded(stdout, stdout_limit),
            read_bounded(stderr, stderr_limit),
            async {
                child
                    .wait()
                    .await
                    .map_err(|error| format!("等待 OCR 退出失败: {error}"))
            }
        )
    })
    .await;
    match result {
        Ok(Ok(((), stdout, stderr, status))) => {
            if !status.success() {
                return Err(format!(
                    "tesseract 执行失败: {}",
                    String::from_utf8_lossy(&stderr).trim()
                ));
            }
            String::from_utf8(stdout)
                .map(|text| text.trim().to_string())
                .map_err(|_| "OCR 输出不是有效 UTF-8".into())
        }
        failure => {
            // 超时/输出超限/管道失败都先终止并回收，再把错误交给 runtime 释放许可。
            let _ = child.kill().await;
            let _ = child.wait().await;
            match failure {
                Err(_) => Err("OCR 识别超时，请重试或缩小识别区域".to_string()),
                Ok(Err(error)) => Err(error),
                _ => unreachable!(),
            }
        }
    }
}

/// 让运行中的进程拥有许可直到 kill/wait 完成，即使唯一调用者取消等待。
pub(crate) async fn recognize_image(png_bytes: Vec<u8>) -> OcrResult {
    recognize_snapshot(png_bytes)
        .await
        .map(|result| result.text)
}

/// 兼容已经持有冻结图像的调用；队列入口限制持有图片的任务总数。
pub(crate) async fn recognize_clip<F>(id: i64, png_bytes: Vec<u8>, cache: F) -> OcrResult
where
    F: FnOnce(&str) -> Result<(), String> + Send + 'static,
{
    recognize_clip_lazy(id, move || Ok(png_bytes), cache).await
}

/// 预览先加入 single-flight/排队，取得许可后才加载 PNG，等待者不持有 BLOB。
pub(crate) async fn recognize_clip_lazy<L, F>(id: i64, load: L, cache: F) -> OcrResult
where
    L: FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    F: FnOnce(&str) -> Result<(), String> + Send + 'static,
{
    recognize_clip_result_lazy(id, load, cache)
        .await
        .map(|result| result.text)
}

/// 结构化侧栏入口同样先入队再读数据库，所有 String 调用适配同一结果。
pub(crate) async fn recognize_clip_result_lazy<L, F>(id: i64, load: L, cache: F) -> StructuredResult
where
    L: FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    F: FnOnce(&str) -> Result<(), String> + Send + 'static,
{
    let configuration = prepare_configuration().await?;
    let runtime = ocr_runtime();
    let key = format!("clip:{id}:{}", configuration.identity);
    let cancellation_key = key.clone();
    runtime
        .run_structured(key, move || async move {
            let png = tauri::async_runtime::spawn_blocking(load)
                .await
                .map_err(|error| format!("OCR 图片读取线程异常: {error}"))??;
            let result = enhanced::recognize_owned(Arc::new(png), configuration, move || {
                !runtime.has_consumers(&cancellation_key)
            })
            .await?;
            cache(&result.text)?;
            Ok(result)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn resolver_skips_broken_candidates_and_returns_the_executable_it_probed() {
        let candidates = ["missing", "broken", "working"]
            .into_iter()
            .map(PathBuf::from);
        let mut probed = Vec::new();

        let resolved = first_available(candidates, |path| {
            probed.push(path.to_path_buf());
            path == Path::new("working")
        });

        assert_eq!(resolved.as_deref(), Some(Path::new("working")));
        assert_eq!(
            probed,
            ["missing", "broken", "working"]
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn explicit_override_is_first_and_duplicate_candidates_are_removed() {
        let override_path = PathBuf::from("tesseract");
        let candidates = tesseract_candidates(Some(override_path.clone().into_os_string()));

        assert_eq!(candidates.first(), Some(&override_path));
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| *candidate == &override_path)
                .count(),
            1
        );
    }

    #[test]
    fn resolver_returns_none_when_every_probe_fails() {
        let resolved = first_available([PathBuf::from("missing")], |_| false);
        assert_eq!(resolved, None);
    }

    #[test]
    fn executable_cache_caches_success_and_failure_until_invalidated() {
        let cache = ExecutableCache::default();
        let calls = AtomicUsize::new(0);
        let resolve = || {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(PathBuf::from("working"))
        };
        assert_eq!(cache.resolve_with(resolve), Some(PathBuf::from("working")));
        assert_eq!(cache.resolve_with(resolve), Some(PathBuf::from("working")));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        cache.invalidate();
        assert_eq!(cache.resolve_with(|| None), None);
        assert_eq!(cache.resolve_with(|| Some(PathBuf::from("late"))), None);
        cache.invalidate();
        assert_eq!(
            cache.resolve_with(|| Some(PathBuf::from("late"))),
            Some(PathBuf::from("late"))
        );
    }

    #[test]
    fn invalidation_during_probe_discards_the_old_generation_result() {
        use std::sync::mpsc;

        let cache = Arc::new(ExecutableCache::default());
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_cache = Arc::clone(&cache);
        let worker_calls = Arc::clone(&calls);
        let worker = std::thread::spawn(move || {
            worker_cache.resolve_with(|| {
                let call = worker_calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    started_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                    Some(PathBuf::from("old"))
                } else {
                    Some(PathBuf::from("new"))
                }
            })
        });

        started_rx.recv().unwrap();
        cache.invalidate();
        resume_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), Some(PathBuf::from("new")));
        assert_eq!(
            cache.resolve_with(|| Some(PathBuf::from("wrong"))),
            Some(PathBuf::from("new"))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn same_clip_uses_one_in_flight_task() {
        let runtime = Box::leak(Box::new(OcrRuntime::new(1)));
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let first = runtime.run_clip(7, move || async move {
            first_calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok("shared".to_string())
        });
        let second_calls = Arc::clone(&calls);
        let second = runtime.run_clip(7, move || async move {
            second_calls.fetch_add(1, Ordering::SeqCst);
            Ok("duplicate".to_string())
        });
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.unwrap(), "shared");
        assert_eq!(second.unwrap(), "shared");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keyed_and_unkeyed_jobs_share_one_global_permit() {
        let runtime = Box::leak(Box::new(OcrRuntime::new(1)));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let work = |active: Arc<AtomicUsize>, peak: Arc<AtomicUsize>| async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            Ok("ok".to_string())
        };
        let keyed = runtime.run_clip(1, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move || work(active, peak)
        });
        let unkeyed = runtime.run_image({
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move || work(active, peak)
        });
        let different_key = runtime.run_clip(2, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move || work(active, peak)
        });
        let (keyed, unkeyed, different_key) = tokio::join!(keyed, unkeyed, different_key);
        keyed.unwrap();
        unkeyed.unwrap();
        different_key.unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }
}

#[cfg(all(test, unix))]
mod process_tests;

#[cfg(test)]
mod structured_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn structured_single_flight_keeps_arc_and_queued_loading_bounded() {
        let runtime: &'static OcrRuntime = Box::leak(Box::new(OcrRuntime::new(1)));
        let gate = Arc::new(tokio::sync::Notify::new());
        let entered = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let png = Arc::new(vec![0u8; 1024]);
        let mut handles = Vec::new();
        for _ in 0..2 {
            let gate = Arc::clone(&gate);
            let entered = Arc::clone(&entered);
            let calls = Arc::clone(&calls);
            let frozen = Arc::clone(&png);
            handles.push(tokio::spawn(runtime.run_structured(
                "same".into(),
                move || async move {
                    let _frozen = frozen;
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.notify_one();
                    gate.notified().await;
                    Ok(StructuredOcr::tesseract(1, 1, "done".into(), None))
                },
            )));
        }
        entered.notified().await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(Arc::strong_count(&png), 2, "合并消费者已释放其Arc");
        let load_count = Arc::new(AtomicUsize::new(0));
        let mut queued = Vec::new();
        for key in ["second", "third"] {
            let load_count = Arc::clone(&load_count);
            queued.push(tokio::spawn(runtime.run_structured(
                key.into(),
                move || async move {
                    load_count.fetch_add(1, Ordering::SeqCst);
                    Ok(StructuredOcr::tesseract(1, 1, "queued".into(), None))
                },
            )));
        }
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(load_count.load(Ordering::SeqCst), 0);
        let rejected = runtime
            .run_structured("fourth".into(), || async {
                panic!("满队列不得加载PNG")
            })
            .await;
        assert!(rejected.unwrap_err().contains("队列"));
        gate.notify_one();
        for handle in handles {
            assert_eq!(handle.await.unwrap().unwrap().text, "done");
        }
        for handle in queued {
            handle.await.unwrap().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(load_count.load(Ordering::SeqCst), 2);
        assert_eq!(Arc::strong_count(&png), 1);
    }
    #[tokio::test]
    async fn structured_waiter_cap_and_abandoned_queue_skip_loading() {
        let runtime: &'static OcrRuntime = Box::leak(Box::new(OcrRuntime::new(1)));
        let permit = runtime.permits.acquire().await.unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let mut jobs = Vec::new();
        for _ in 0..64 {
            let loads = Arc::clone(&loads);
            jobs.push(tokio::spawn(runtime.run_structured(
                "shared".into(),
                move || async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(StructuredOcr::tesseract(1, 1, "shared".into(), None))
                },
            )));
        }
        let abandoned = tokio::spawn(runtime.run_structured("abandoned".into(), || async {
            panic!("无人等待的排队请求不得加载PNG")
        }));
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert!(runtime
            .run_structured("shared".into(), || async {
                panic!("不启动第65个等待者")
            })
            .await
            .unwrap_err()
            .contains("等待者"));
        abandoned.abort();
        let _ = abandoned.await;
        for job in jobs.drain(..63) {
            job.abort();
            let _ = job.await;
        }
        drop(permit);
        assert_eq!(jobs.pop().unwrap().await.unwrap().unwrap().text, "shared");
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert!(runtime.snapshots.lock().unwrap().is_empty());
    }
}
