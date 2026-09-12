mod changes;
pub(crate) mod content;
mod poll_state;
#[cfg(target_os = "linux")]
mod tmux;
mod wake;
mod writer;

pub use writer::{
    clipboard_set_html_with_retry, clipboard_set_image_with_retry, clipboard_set_text_with_retry,
};

use crate::models::{AppConfig, ContentType};
use crate::storage::StorageEngine;
use arboard::Clipboard;
use content::{
    compute_hash, encode_image_to_png, is_sensitive_text, rgba_fingerprint, strip_html_tags,
    validate_image_layout,
};
use poll_state::PollState;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 无平台变更信号时的回退周期；程序化写入通过 wake::nudge 提前唤醒。
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
struct WriteEpoch {
    generation: u64,
    hashes: Vec<String>,
}

type SuppressedHashes = Arc<Mutex<WriteEpoch>>;

pub struct ClipboardWatcher {
    running: Arc<Mutex<bool>>,
    /// 写入与登记在同一临界区；读取用前后版本校验，不能持锁等待系统图片。
    suppressed_hashes: SuppressedHashes,
}

impl ClipboardWatcher {
    pub fn new() -> Self {
        Self {
            running: Arc::new(Mutex::new(false)),
            suppressed_hashes: Arc::new(Mutex::new(WriteEpoch::default())),
        }
    }

    /// 写入成功才登记本次快照；失败保留先前有效登记。锁内没有轮询等待或数据库操作。
    pub(crate) fn write_suppressed(
        &self,
        hashes: Vec<String>,
        write: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut suppressed = self
            .suppressed_hashes
            .lock()
            .map_err(|error| error.to_string())?;
        write()?;
        suppressed.generation = suppressed.generation.wrapping_add(1);
        suppressed.hashes = hashes;
        Ok(())
    }

    pub fn start(
        &self,
        app_handle: AppHandle,
        storage: Arc<Mutex<StorageEngine>>,
        config: Arc<Mutex<AppConfig>>,
    ) {
        let running = Arc::clone(&self.running);
        let suppressed_hashes = Arc::clone(&self.suppressed_hashes);
        {
            let mut active = running.lock().unwrap_or_else(|error| error.into_inner());
            if *active {
                return;
            }
            *active = true;
        }
        thread::spawn(move || {
            let mut clipboard = match Clipboard::new() {
                Ok(clipboard) => clipboard,
                Err(error) => {
                    log::error!("剪贴板初始化失败: {error}");
                    *running.lock().unwrap_or_else(|error| error.into_inner()) = false;
                    return;
                }
            };
            let mut poll_state = PollState::default();
            let mut changes = changes::ChangeMonitor::new();
            let mut observed_write_generation = None;
            let mut read_failure = PollState::default();
            let mut last_rejected_image_layout = None;
            let mut sensitive_check_counter = 0u32;
            const SENSITIVE_TTL_SECS: i64 = 300;
            const SENSITIVE_CHECK_INTERVAL: u32 = 60;

            #[cfg(target_os = "linux")]
            let tmux_last_hash = {
                let initial_hash = std::fs::read_to_string(crate::commands::tmux_buf_path())
                    .ok()
                    .filter(|s| !s.is_empty())
                    .map(|s| compute_hash(s.as_bytes()))
                    .unwrap_or_default();
                Arc::new(Mutex::new(initial_hash))
            };
            #[cfg(not(target_os = "linux"))]
            let tmux_last_hash = Arc::new(Mutex::new(String::new()));
            #[cfg(target_os = "linux")]
            {
                let running = Arc::clone(&running);
                let config = Arc::clone(&config);
                let storage = Arc::clone(&storage);
                let app_handle = app_handle.clone();
                let tmux_last_hash = Arc::clone(&tmux_last_hash);
                thread::spawn(move || {
                    tmux::start(running, config, storage, app_handle, tmux_last_hash)
                });
            }
            log::info!("剪贴板监听器已启动");
            loop {
                if !*running.lock().unwrap_or_else(|error| error.into_inner()) {
                    break;
                }
                sensitive_check_counter += 1;
                if sensitive_check_counter >= SENSITIVE_CHECK_INTERVAL {
                    sensitive_check_counter = 0;
                    if let Ok(storage) = storage.lock() {
                        match storage.purge_expired_sensitive(SENSITIVE_TTL_SECS) {
                            Ok(ids) => {
                                for id in ids {
                                    let _ = app_handle.emit("clip-removed", id);
                                }
                            }
                            Err(error) => log::warn!("敏感历史清理失败: {error}"),
                        }
                    }
                }
                let generation = suppressed_hashes
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .generation;
                let (changed, reliable) = changes.poll();
                let now = Instant::now();
                if reliable && changed {
                    // 同一个 owner 重新设定 selection 也会收到 XFixes 事件；同图重新复制应置顶。
                    poll_state.reset();
                    read_failure.reset();
                }
                if !changed
                    && observed_write_generation == Some(generation)
                    && !poll_state.needs_retry(now)
                    && !read_failure.needs_retry(now)
                {
                    wake::wait_for_next_poll(POLL_INTERVAL);
                    continue;
                }
                // 前后短锁版本校验；4s 的系统读取不会阻塞 copy_text 写入。
                let snapshot = ClipboardSnapshot::read(&mut clipboard);
                let suppressed = {
                    let mut guard = suppressed_hashes
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if guard.generation != generation {
                        // 新写入也已 nudge；无需再等一整个周期。
                        continue;
                    }
                    if snapshot.is_some() {
                        std::mem::take(&mut guard.hashes)
                    } else {
                        Vec::new()
                    }
                };
                observed_write_generation = Some(generation);
                if snapshot.is_none() {
                    read_failure.observe("unavailable", false, now);
                    read_failure.failed(now);
                } else {
                    read_failure.settle();
                }
                let last_tmux_hash = tmux_last_hash
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if let Some(snapshot) = snapshot {
                    if let Some(prepared) = prepare_snapshot(
                        snapshot,
                        &suppressed,
                        &last_tmux_hash,
                        &mut poll_state,
                        &mut last_rejected_image_layout,
                        Instant::now(),
                    ) {
                        let max_history = config
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .max_history;
                        let result = {
                            let storage = storage.lock().unwrap_or_else(|error| error.into_inner());
                            storage
                                .insert_clip(
                                    &prepared.kind,
                                    prepared.text.as_deref(),
                                    prepared.html.as_deref(),
                                    prepared.png.as_deref(),
                                    &prepared.hash,
                                    prepared.byte_size,
                                    prepared.sensitive,
                                )
                                .map(|clip| {
                                    let removed = match storage.cleanup_old_entries(max_history) {
                                        Ok(ids) => ids,
                                        Err(error) => {
                                            log::warn!("历史容量清理失败: {error}");
                                            Vec::new()
                                        }
                                    };
                                    (clip.without_image_data(), removed)
                                })
                        };
                        match result {
                            Ok((clip, removed)) => {
                                poll_state.settle();
                                for id in removed {
                                    let _ = app_handle.emit("clip-removed", id);
                                }
                                let _ = app_handle.emit("clip-added", &clip);
                                log::debug!(
                                    "新剪贴板内容，类型: {}, 大小: {} 字节",
                                    prepared.kind.as_str(),
                                    prepared.byte_size
                                );
                            }
                            Err(error) => {
                                poll_state.failed(Instant::now());
                                crate::error::report("剪贴板内容保存失败，稍后重试", error);
                            }
                        }
                    }
                }
                wake::wait_for_next_poll(POLL_INTERVAL);
            }
            log::info!("剪贴板监听器已停止");
        });
    }
}

impl Default for ClipboardWatcher {
    fn default() -> Self {
        Self::new()
    }
}

enum ClipboardSnapshot {
    Text(String),
    Html { html: String, text: String },
    Image(arboard::ImageData<'static>),
}

impl ClipboardSnapshot {
    fn read(clipboard: &mut Clipboard) -> Option<Self> {
        if let Ok(html) = clipboard.get().html() {
            if !html.is_empty() {
                let text = clipboard
                    .get_text()
                    .unwrap_or_else(|_| strip_html_tags(&html));
                return Some(Self::Html { html, text });
            }
        }
        if let Ok(text) = clipboard.get_text() {
            if !text.is_empty() {
                return Some(Self::Text(text));
            }
        }
        clipboard.get_image().ok().map(Self::Image)
    }
}

struct PreparedContent {
    kind: ContentType,
    text: Option<String>,
    html: Option<String>,
    png: Option<Vec<u8>>,
    hash: String,
    byte_size: i64,
    sensitive: bool,
}

fn prepare_snapshot(
    snapshot: ClipboardSnapshot,
    suppressed: &[String],
    tmux_hash: &str,
    state: &mut PollState,
    rejected_layout: &mut Option<(usize, usize, usize)>,
    now: Instant,
) -> Option<PreparedContent> {
    let (kind, text, html, png, hash, byte_size) = match snapshot {
        ClipboardSnapshot::Text(text) => {
            *rejected_layout = None;
            let hash = compute_hash(text.as_bytes());
            let size = text.len() as i64;
            if !state.observe(
                &format!("text:{hash}"),
                suppressed.contains(&hash) || hash == tmux_hash,
                now,
            ) {
                return None;
            }
            (ContentType::Text, Some(text), None, None, hash, size)
        }
        ClipboardSnapshot::Html { html, text } => {
            *rejected_layout = None;
            let hash = compute_hash(html.as_bytes());
            let size = html.len() as i64;
            if !state.observe(
                &format!("html:{hash}"),
                suppressed.contains(&hash) || hash == tmux_hash,
                now,
            ) {
                return None;
            }
            (ContentType::Html, Some(text), Some(html), None, hash, size)
        }
        ClipboardSnapshot::Image(image) => {
            if let Err(error) = validate_image_layout(image.width, image.height, image.bytes.len())
            {
                let layout = (image.width, image.height, image.bytes.len());
                if *rejected_layout != Some(layout) {
                    log::warn!("忽略异常剪贴板图片: {error}");
                }
                *rejected_layout = Some(layout);
                state.reset();
                return None;
            }
            *rejected_layout = None;
            let identity = format!(
                "image:{}",
                rgba_fingerprint(image.width, image.height, &image.bytes)
            );
            // 指纹只短路已成功保存/明确抑制的图片；失败按同一状态机退避，编码也不空转。
            if !state.observe(&identity, false, now) {
                return None;
            }
            let Some(png) = encode_image_to_png(&image) else {
                state.failed(now);
                return None;
            };
            let hash = compute_hash(&png);
            if suppressed.contains(&hash) {
                state.settle();
                return None;
            }
            let size = png.len() as i64;
            (ContentType::Image, None, None, Some(png), hash, size)
        }
    };
    let sensitive = text.as_deref().is_some_and(is_sensitive_text);
    Some(PreparedContent {
        kind,
        text,
        html,
        png,
        hash,
        byte_size,
        sensitive,
    })
}

#[cfg(test)]
mod tests;
