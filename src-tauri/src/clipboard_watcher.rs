use crate::models::{AppConfig, ContentType};
use crate::storage::StorageEngine;
use arboard::Clipboard;
use image::{ImageBuffer, RgbaImage};
use inotify::{Inotify, WatchMask};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// 简单去除 HTML 标签，用于生成 FTS 可搜索的纯文本
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

/// 检测文本是否可能包含敏感内容（密码、Token、API Key 等）
fn is_sensitive_text(text: &str) -> bool {
    // 仅检测纯文本，短于 8 字符的不检测（太短不像 token）
    if text.len() < 8 {
        return false;
    }
    // 常见 API Key / Token 前缀
    const PREFIXES: &[&str] = &[
        "sk-",         // OpenAI
        "sk_live_",    // Stripe
        "sk_test_",    // Stripe
        "ghp_",        // GitHub PAT
        "gho_",        // GitHub OAuth
        "ghu_",        // GitHub User-to-server
        "ghs_",        // GitHub Server-to-server
        "github_pat_", // GitHub Fine-grained PAT
        "AKIA",        // AWS Access Key
        "Bearer ",     // Bearer Token
        "eyJ",         // JWT (base64 of {"...)
        "xox",         // Slack (xoxb-, xoxp-, xoxs-)
        "glpat-",      // GitLab PAT
        "npm_",        // npm token
        "pypi-",       // PyPI token
    ];
    for prefix in PREFIXES {
        if text.starts_with(prefix) {
            return true;
        }
    }
    // 通用 password/secret 关键字检测（key=value 格式）
    let lower = text.to_lowercase();
    if (lower.contains("password") || lower.contains("passwd") || lower.contains("secret"))
        && (lower.contains('=') || lower.contains(':'))
    {
        return true;
    }
    false
}

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
                    start_tmux_watcher(running, config, storage, app_handle, tmux_last_hash);
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
                                        log::warn!("剪贴板 HTML 保存失败: {}", e);
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
                                        log::warn!("剪贴板内容保存失败: {}", e);
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
                                            log::warn!("剪贴板图片保存失败: {}", e);
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

fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// 将 arboard 的 RGBA 图片数据编码为 PNG 字节
fn encode_image_to_png(img: &arboard::ImageData) -> Option<Vec<u8>> {
    let buffer: RgbaImage =
        ImageBuffer::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec())?;
    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    if let Err(e) = buffer.write_to(&mut cursor, image::ImageFormat::Png) {
        log::warn!("PNG 编码失败: {}", e);
        return None;
    }
    Some(png_bytes)
}

/// tmux 缓冲区 inotify 监听线程
/// 使用 inotify 监控 tmux-buf 文件的 CLOSE_WRITE 事件，实现零轮询的即时捕获
fn start_tmux_watcher(
    running: Arc<Mutex<bool>>,
    config: Arc<Mutex<AppConfig>>,
    storage: Arc<Mutex<StorageEngine>>,
    app_handle: AppHandle,
    tmux_last_hash: Arc<Mutex<String>>,
) {
    let tmux_buf_path = crate::commands::tmux_buf_path();
    let mut hook_check_counter: u32 = 0;
    const HOOK_CHECK_INTERVAL: u32 = 60; // 每 60 次唤醒检查一次 hook（inotify 超时为 1s）

    // 确保监控目录存在
    if let Some(parent) = tmux_buf_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 初始化 inotify，监控文件所在目录
    let mut inotify = match Inotify::init() {
        Ok(i) => i,
        Err(e) => {
            log::error!("inotify 初始化失败，tmux 监听不可用: {}", e);
            return;
        }
    };

    // 监控目录而非文件本身（因为 tmux save-buffer 可能用 rename/create 替换文件）
    let watch_dir = tmux_buf_path.parent().unwrap();
    if let Err(e) = inotify.watches().add(
        watch_dir,
        WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE,
    ) {
        log::error!("inotify watch 添加失败: {}", e);
        return;
    }

    let target_filename = tmux_buf_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    log::info!("tmux inotify 监听已启动: {:?}", tmux_buf_path);

    let mut buf = [0u8; 4096];
    loop {
        // 检查是否应停止
        {
            let r = running.lock().unwrap_or_else(|e| e.into_inner());
            if !*r {
                break;
            }
        }

        // 检查 tmux_capture 是否仍然启用
        let tmux_enabled = config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tmux_capture;
        if !tmux_enabled {
            thread::sleep(Duration::from_secs(2));
            continue;
        }

        // 定期验证 copy-pipe 绑定存在性
        hook_check_counter += 1;
        if hook_check_counter >= HOOK_CHECK_INTERVAL {
            hook_check_counter = 0;
            let check = std::process::Command::new("tmux")
                .args(["list-keys", "-T", "copy-mode-vi"])
                .output();
            let binding_missing = match &check {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    !stdout.contains("copy-pipe-and-cancel")
                }
                Err(_) => false,
            };
            if binding_missing {
                log::warn!("tmux copy-pipe 绑定丢失，正在重建...");
                if let Err(e) = crate::commands::setup_tmux_hook() {
                    log::warn!("tmux 绑定重建失败: {}", e);
                }
            }
        }

        // 等待 inotify 事件（poll 超时 1 秒，确保能响应 running 状态变化和 hook 检查）
        let fd = inotify.as_raw_fd();
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let poll_ret = unsafe { libc::poll(&mut pollfd, 1, 1000) };
        if poll_ret <= 0 {
            // 超时或错误，回到循环顶部检查 running / hook
            continue;
        }

        let events = match inotify.read_events(&mut buf) {
            Ok(events) => events,
            Err(_) => continue,
        };

        let mut file_changed = false;
        for event in events {
            if let Some(name) = event.name {
                if name.to_string_lossy() == target_filename {
                    file_changed = true;
                    break;
                }
            }
        }

        if !file_changed {
            continue;
        }

        // 短暂等待确保文件写入完毕
        thread::sleep(Duration::from_millis(10));

        // 读取文件内容并处理
        let content = match std::fs::read_to_string(&tmux_buf_path) {
            Ok(c) if !c.is_empty() => c,
            _ => continue,
        };

        let hash = compute_hash(content.as_bytes());
        let current_tmux_hash = tmux_last_hash
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        if hash == current_tmux_hash {
            continue;
        }

        // 更新共享 hash
        *tmux_last_hash.lock().unwrap_or_else(|e| e.into_inner()) = hash.clone();

        let max_history = config.lock().unwrap_or_else(|e| e.into_inner()).max_history;
        let byte_size = content.len() as i64;
        let sensitive = is_sensitive_text(&content);

        let result = {
            let storage = storage.lock().unwrap_or_else(|e| e.into_inner());
            match storage.insert_clip(
                &ContentType::Text,
                Some(&content),
                None,
                None,
                &hash,
                byte_size,
                sensitive,
            ) {
                Ok(clip) => {
                    let removed = storage.cleanup_old_entries(max_history).ok();
                    Some((clip, removed))
                }
                Err(e) => {
                    log::warn!("tmux 缓冲区保存失败: {}", e);
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
            log::debug!("tmux 缓冲区内容（inotify），大小: {} 字节", byte_size);
        }
    }

    log::info!("tmux inotify 监听已停止");
}
