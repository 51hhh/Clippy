pub(crate) mod content;
mod tmux;
mod writer;

pub use writer::{
    clipboard_set_html_with_retry, clipboard_set_image_with_retry, clipboard_set_text_with_retry,
};

use crate::models::{AppConfig, ContentType};
use crate::storage::StorageEngine;
use arboard::Clipboard;
use content::{compute_hash, encode_image_to_png, is_sensitive_text, strip_html_tags};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub struct ClipboardWatcher {
    running: Arc<Mutex<bool>>,
    /// select_clip 写入剪贴板时设置此哈希，watcher 遇到相同哈希时跳过
    skip_hash: Arc<Mutex<Option<String>>>,
}

impl ClipboardWatcher {
    pub fn new() -> Self {
        Self {
            running: Arc::new(Mutex::new(false)),
            skip_hash: Arc::new(Mutex::new(None)),
        }
    }

    /// 让 watcher 跳过下一次检测到的指定哈希（由 select_clip 调用）
    pub fn set_skip_hash(&self, hash: String) {
        *self.skip_hash.lock().unwrap_or_else(|e| e.into_inner()) = Some(hash);
    }

    pub fn start(
        &self,
        app_handle: AppHandle,
        storage: Arc<Mutex<StorageEngine>>,
        config: Arc<Mutex<AppConfig>>,
    ) {
        let running = Arc::clone(&self.running);
        let skip_hash = Arc::clone(&self.skip_hash);
        {
            let mut r = running.lock().unwrap_or_else(|e| e.into_inner());
            if *r {
                return;
            }
            *r = true;
        }

        thread::spawn(move || {
            let mut clipboard = match Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("剪贴板初始化失败: {}", e);
                    return;
                }
            };

            let mut last_hash = String::new();
            let mut sensitive_check_counter: u32 = 0;
            const SENSITIVE_TTL_SECS: i64 = 300; // 5 分钟后自动删除敏感条目
            const SENSITIVE_CHECK_INTERVAL: u32 = 60; // 每 60 次循环（~30秒）检查一次

            // tmux inotify 线程共享的 last_tmux_hash
            let tmux_last_hash: Arc<Mutex<String>> = {
                let tmux_buf_path = crate::commands::tmux_buf_path();
                let initial_hash = std::fs::read_to_string(&tmux_buf_path)
                    .ok()
                    .filter(|s| !s.is_empty())
                    .map(|s| compute_hash(s.as_bytes()))
                    .unwrap_or_default();
                Arc::new(Mutex::new(initial_hash))
            };

            // 启动 tmux inotify 监听线程
            {
                let running = Arc::clone(&running);
                let config = Arc::clone(&config);
                let storage = Arc::clone(&storage);
                let app_handle = app_handle.clone();
                let tmux_last_hash = Arc::clone(&tmux_last_hash);
                thread::spawn(move || {
                    tmux::start(running, config, storage, app_handle, tmux_last_hash);
                });
            }

            log::info!("剪贴板监听器已启动");

            loop {
                {
                    let r = running.lock().unwrap_or_else(|e| e.into_inner());
                    if !*r {
                        break;
                    }
                }

                // 读取 tmux 线程最新的 hash，防止剪贴板检测重复捕获 tmux 内容
                let last_tmux_hash = tmux_last_hash
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();

                // 定时清理过期敏感条目
                sensitive_check_counter += 1;
                if sensitive_check_counter >= SENSITIVE_CHECK_INTERVAL {
                    sensitive_check_counter = 0;
                    if let Ok(storage) = storage.lock() {
                        if let Ok(removed_ids) = storage.purge_expired_sensitive(SENSITIVE_TTL_SECS)
                        {
                            for rid in &removed_ids {
                                let _ = app_handle.emit("clip-removed", rid);
                            }
                            if !removed_ids.is_empty() {
                                log::debug!("清理 {} 条过期敏感条目", removed_ids.len());
                            }
                        }
                    }
                }

                // 优先检测 HTML 富文本（浏览器复制的内容同时有 HTML 和纯文本）
                if let Ok(html) = clipboard.get().html() {
                    if !html.is_empty() {
                        let hash = compute_hash(html.as_bytes());
                        if hash != last_hash && hash != last_tmux_hash {
                            {
                                let mut skip = skip_hash.lock().unwrap_or_else(|e| e.into_inner());
                                if skip.as_deref() == Some(&hash) {
                                    *skip = None;
                                    thread::sleep(Duration::from_millis(500));
                                    continue;
                                }
                            }
                            last_hash = hash.clone();

                            // 获取纯文本回退（用于搜索和预览）
                            let text_fallback = clipboard.get_text().ok().or_else(|| {
                                // 剪贴板无纯文本时，从 HTML 中提取可搜索文本
                                Some(strip_html_tags(&html))
                            });

                            let max_history =
                                config.lock().unwrap_or_else(|e| e.into_inner()).max_history;
                            let byte_size = html.len() as i64;
                            let sensitive = text_fallback.as_deref().is_some_and(is_sensitive_text);

                            let result = {
                                let storage = storage.lock().unwrap_or_else(|e| e.into_inner());
                                let clip_result = storage.insert_clip(
                                    &ContentType::Html,
                                    text_fallback.as_deref(),
                                    Some(&html),
                                    None,
                                    &hash,
                                    byte_size,
                                    sensitive,
                                );
                                match clip_result {
                                    Ok(clip) => {
                                        let removed = storage.cleanup_old_entries(max_history).ok();
                                        Some((clip, removed))
                                    }
                                    Err(e) => {
                                        crate::error::report("剪贴板 HTML 保存失败", e);
                                        None
                                    }
                                }
                            };

                            if let Some((clip, removed)) = result {
                                if let Some(removed_ids) = removed {
                                    for rid in removed_ids {
                                        let _ = app_handle.emit("clip-removed", rid);
                                    }
                                }
                                let _ = app_handle.emit("clip-added", &clip);
                                log::debug!("新剪贴板内容，类型: html, 大小: {} 字节", byte_size);
                            }
                        }

                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                }

                if let Ok(text) = clipboard.get_text() {
                    if !text.is_empty() {
                        let hash = compute_hash(text.as_bytes());
                        if hash != last_hash && hash != last_tmux_hash {
                            // 跳过 select_clip 写入的内容
                            {
                                let mut skip = skip_hash.lock().unwrap_or_else(|e| e.into_inner());
                                if skip.as_deref() == Some(&hash) {
                                    *skip = None;
                                    thread::sleep(Duration::from_millis(500));
                                    continue;
                                }
                            }
                            last_hash = hash.clone();

                            // 先读取 config，再锁 storage，缩小锁范围
                            let max_history =
                                config.lock().unwrap_or_else(|e| e.into_inner()).max_history;
                            let byte_size = text.len() as i64;
                            let sensitive = is_sensitive_text(&text);

                            let result = {
                                let storage = storage.lock().unwrap_or_else(|e| e.into_inner());
                                let clip_result = storage.insert_clip(
                                    &ContentType::Text,
                                    Some(&text),
                                    None,
                                    None,
                                    &hash,
                                    byte_size,
                                    sensitive,
                                );
                                match clip_result {
                                    Ok(clip) => {
                                        let removed = storage.cleanup_old_entries(max_history).ok();
                                        Some((clip, removed))
                                    }
                                    Err(e) => {
                                        crate::error::report("剪贴板内容保存失败", e);
                                        None
                                    }
                                }
                            }; // storage lock released here

                            if let Some((clip, removed)) = result {
                                if let Some(removed_ids) = removed {
                                    for rid in removed_ids {
                                        let _ = app_handle.emit("clip-removed", rid);
                                    }
                                }
                                let _ = app_handle.emit("clip-added", &clip);
                                log::debug!("新剪贴板内容，类型: text, 大小: {} 字节", byte_size);
                            }
                        }

                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                }

                // 无文本内容时尝试获取图片
                if let Ok(img) = clipboard.get_image() {
                    if !img.bytes.is_empty() {
                        if let Some(png_bytes) = encode_image_to_png(&img) {
                            let hash = compute_hash(&png_bytes);
                            if hash != last_hash && hash != last_tmux_hash {
                                // 跳过 select_clip 写入的图片
                                {
                                    let mut skip =
                                        skip_hash.lock().unwrap_or_else(|e| e.into_inner());
                                    if skip.as_deref() == Some(&hash) {
                                        *skip = None;
                                        thread::sleep(Duration::from_millis(500));
                                        continue;
                                    }
                                }
                                last_hash = hash.clone();

                                let max_history =
                                    config.lock().unwrap_or_else(|e| e.into_inner()).max_history;
                                let byte_size = png_bytes.len() as i64;

                                let result = {
                                    let storage = storage.lock().unwrap_or_else(|e| e.into_inner());
                                    let clip_result = storage.insert_clip(
                                        &ContentType::Image,
                                        None,
                                        None,
                                        Some(&png_bytes),
                                        &hash,
                                        byte_size,
                                        false,
                                    );
                                    match clip_result {
                                        Ok(mut clip) => {
                                            // 事件中不携带图片数据，前端按需加载
                                            clip.image_data = None;
                                            let removed =
                                                storage.cleanup_old_entries(max_history).ok();
                                            Some((clip, removed))
                                        }
                                        Err(e) => {
                                            crate::error::report("剪贴板图片保存失败", e);
                                            None
                                        }
                                    }
                                };

                                if let Some((clip, removed)) = result {
                                    if let Some(removed_ids) = removed {
                                        for rid in removed_ids {
                                            let _ = app_handle.emit("clip-removed", rid);
                                        }
                                    }
                                    let _ = app_handle.emit("clip-added", &clip);
                                    log::debug!(
                                        "新剪贴板内容，类型: image, 大小: {} 字节",
                                        byte_size
                                    );
                                }
                            }
                        }
                    }
                }

                thread::sleep(Duration::from_millis(500));
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
