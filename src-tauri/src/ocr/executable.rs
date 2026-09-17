//! Tesseract 可执行文件发现、限时探测和可失效缓存。

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

const TESSERACT_PATH_ENV: &str = "CLIPPY_TESSERACT_PATH";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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

pub(super) fn probe_command_with_timeout(mut command: Command, timeout: Duration) -> bool {
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
pub(super) struct ExecutableCache {
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
    pub(super) fn resolve_with<F>(&self, resolver: F) -> Option<PathBuf>
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

pub(super) fn tesseract_executable() -> Option<PathBuf> {
    tesseract_executable_until(Instant::now() + PROBE_TIMEOUT)
}

pub(super) fn tesseract_executable_until(deadline: Instant) -> Option<PathBuf> {
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
pub(super) fn invalidate_executable_cache() {
    executable_cache().invalidate();
}

#[cfg(target_os = "linux")]
pub(super) fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未安装 tesseract。请运行 sudo apt install tesseract-ocr tesseract-ocr-chi-sim"
}

#[cfg(target_os = "windows")]
pub(super) fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract.exe。请安装 Tesseract 后重启 Clippy，或设置 CLIPPY_TESSERACT_PATH"
}

#[cfg(target_os = "macos")]
pub(super) fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract。请通过 Homebrew/MacPorts 安装，或设置 CLIPPY_TESSERACT_PATH"
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub(super) fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
}
