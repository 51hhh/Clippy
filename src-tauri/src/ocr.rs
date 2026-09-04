//! ocr.rs — Tesseract OCR 封装
//! 通过命令行调用系统 tesseract，避免编译时动态链接依赖。
//! tesseract 缺失时返回友好错误，不影响应用启动。

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{oneshot, Semaphore};

const TESSERACT_PATH_ENV: &str = "CLIPPY_TESSERACT_PATH";
const OCR_MAX_CONCURRENCY: usize = 1;
type OcrResult = Result<String, String>;

struct OcrRuntime {
    permits: Arc<Semaphore>,
    in_flight: Mutex<HashMap<i64, Vec<oneshot::Sender<OcrResult>>>>,
}

impl OcrRuntime {
    fn new(max_concurrency: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrency)),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    async fn run_image<F, Fut>(&self, work: F) -> OcrResult
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = OcrResult>,
    {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| "OCR 并发控制器已关闭".to_string())?;
        work().await
    }

    async fn run_clip<F, Fut>(&'static self, id: i64, work: F) -> OcrResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = OcrResult> + Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let should_start = {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match in_flight.get_mut(&id) {
                Some(waiters) => {
                    waiters.push(sender);
                    false
                }
                None => {
                    in_flight.insert(id, vec![sender]);
                    true
                }
            }
        };
        if should_start {
            tauri::async_runtime::spawn(async move {
                let result = self.run_image(work).await;
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

fn probe_tesseract(path: &Path) -> bool {
    Command::new(path)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[derive(Default)]
struct ExecutableCache {
    state: Mutex<ExecutableCacheState>,
}

#[derive(Default)]
struct ExecutableCacheState {
    generation: u64,
    value: Option<Option<PathBuf>>,
}

impl ExecutableCache {
    fn resolve_with<F>(&self, mut resolver: F) -> Option<PathBuf>
    where
        F: FnMut() -> Option<PathBuf>,
    {
        loop {
            let generation = {
                let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(cached) = state.value.as_ref() {
                    return cached.clone();
                }
                state.generation
            };

            // 探测会启动外部进程，不能占着同步锁阻塞其它查询或安装后的失效操作。
            let resolved = resolver();
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.generation != generation {
                // 探测期间发生过安装/卸载，旧结果不得覆盖新一代缓存。
                continue;
            }
            if let Some(cached) = state.value.as_ref() {
                return cached.clone();
            }
            state.value = Some(resolved.clone());
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
    executable_cache().resolve_with(|| {
        first_available(
            tesseract_candidates(env::var_os(TESSERACT_PATH_ENV)),
            probe_tesseract,
        )
    })
}

/// 安装流程改变了外部工具状态，下一次查询必须重新探测。
pub(crate) fn invalidate_executable_cache() {
    executable_cache().invalidate();
}

/// 检查系统是否安装了可实际执行的 Tesseract。
pub fn is_available() -> bool {
    tesseract_executable().is_some()
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

/// 对 PNG 图片字节进行 OCR 识别，返回文字内容。
/// 通过 stdin 管道传入图片数据，stdout 获取识别结果。
fn recognize(png_bytes: &[u8]) -> Result<String, String> {
    let executable =
        tesseract_executable().ok_or_else(|| missing_tesseract_message().to_string())?;
    let mut child = Command::new(executable)
        .args(["stdin", "stdout", "-l", "eng+chi_sim"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                // 已缓存的文件可能在运行期间被卸载或替换，下次请求重新解析候选路径。
                invalidate_executable_cache();
                missing_tesseract_message().to_string()
            } else {
                format!("启动 tesseract 失败: {}", e)
            }
        })?;

    // 通过 stdin 传入图片数据
    if let Some(ref mut stdin) = child.stdin {
        stdin
            .write_all(png_bytes)
            .map_err(|e| format!("写入图片数据失败: {}", e))?;
    }
    // 关闭 stdin 触发 tesseract 处理
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 tesseract 结束失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tesseract 执行失败: {}", stderr.trim()));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(text)
}

/// 所有无 clip 身份的 OCR（例如截图选区）也必须经过同一全局资源门。
pub(crate) async fn recognize_image(png_bytes: Vec<u8>) -> OcrResult {
    ocr_runtime()
        .run_image(move || async move {
            tauri::async_runtime::spawn_blocking(move || recognize(&png_bytes))
                .await
                .map_err(|error| format!("OCR 线程异常: {error}"))?
        })
        .await
}

/// 按 clip ID 合并预览与翻译发起的识别，并共用全局资源门。
pub(crate) async fn recognize_clip<F>(id: i64, png_bytes: Vec<u8>, cache: F) -> OcrResult
where
    F: FnOnce(&str) -> Result<(), String> + Send + 'static,
{
    ocr_runtime()
        .run_clip(id, move || async move {
            let text = tauri::async_runtime::spawn_blocking(move || recognize(&png_bytes))
                .await
                .map_err(|error| format!("OCR 线程异常: {error}"))??;
            cache(&text)?;
            Ok(text)
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
