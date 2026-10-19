use super::content::{compute_hash, is_sensitive_text, MAX_CLIPBOARD_TEXT_BYTES};
use super::poll_state::PollState;
use crate::models::{AppConfig, ContentType};
use crate::storage::StorageEngine;
use inotify::{Inotify, WatchMask};
use nix::poll::{poll, PollFd, PollFlags};
use std::io::Read;
use std::os::fd::AsFd;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 使用 inotify 监控 tmux 缓冲区文件，实现零轮询的即时捕获。
pub(super) fn start(
    running: Arc<Mutex<bool>>,
    config: Arc<Mutex<AppConfig>>,
    storage: Arc<Mutex<StorageEngine>>,
    app_handle: AppHandle,
) {
    let tmux_buf_path = crate::commands::tmux_buf_path();
    if let Err(error) = crate::commands::ensure_tmux_state_dir() {
        log::error!("tmux 状态目录不可用: {error}");
        return;
    }
    // 文件去重只属于 tmux 输入：旧缓冲内容不能永久屏蔽独立的系统 selection。
    let mut last_hash = read_bounded_content(&tmux_buf_path)
        .ok()
        .flatten()
        .filter(|text| !text.is_empty())
        .map(|text| compute_hash(text.as_bytes()))
        .unwrap_or_default();
    let mut hook_check_counter: u32 = 0;
    const HOOK_CHECK_INTERVAL: u32 = 60;

    let mut inotify = match Inotify::init() {
        Ok(inotify) => inotify,
        Err(error) => {
            log::error!("inotify 初始化失败，tmux 监听不可用: {}", error);
            return;
        }
    };

    let Some(watch_dir) = tmux_buf_path.parent() else {
        log::error!("tmux 缓冲区路径缺少父目录");
        return;
    };
    if let Err(error) = inotify.watches().add(
        watch_dir,
        WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE,
    ) {
        log::error!("inotify watch 添加失败: {}", error);
        return;
    }

    let target_filename = tmux_buf_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    log::info!("tmux inotify 监听已启动: {:?}", tmux_buf_path);

    let mut buffer = [0u8; 4096];
    let mut state = PollState::default();
    loop {
        if !*running.lock().unwrap_or_else(|error| error.into_inner()) {
            break;
        }

        let tmux_enabled = config
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tmux_capture;
        if !tmux_enabled {
            thread::sleep(Duration::from_secs(2));
            continue;
        }

        hook_check_counter += 1;
        if hook_check_counter >= HOOK_CHECK_INTERVAL {
            hook_check_counter = 0;
            rebuild_missing_hook();
        }

        let mut pollfds = [PollFd::new(inotify.as_fd(), PollFlags::POLLIN)];
        let poll_result = poll(&mut pollfds, 1000_u16).unwrap_or(0);
        let file_changed = if poll_result > 0 {
            match inotify.read_events(&mut buffer) {
                Ok(events) => events
                    .filter_map(|event| event.name)
                    .any(|name| name.to_string_lossy() == target_filename),
                Err(_) => false,
            }
        } else {
            false
        };
        // 写入失败不一定再产生 inotify 事件；超时仍需检查待重试内容。
        if !file_changed && !state.has_pending() {
            continue;
        }

        thread::sleep(Duration::from_millis(10));
        let content = match read_bounded_content(&tmux_buf_path) {
            Ok(Some(content)) if !content.is_empty() => content,
            Ok(Some(_)) => continue,
            Ok(None) => {
                log::warn!("忽略超过 {MAX_CLIPBOARD_TEXT_BYTES} 字节的 tmux 缓冲内容");
                continue;
            }
            _ => continue,
        };
        let hash = compute_hash(content.as_bytes());
        if !state.observe(&hash, hash == last_hash, Instant::now()) {
            continue;
        }
        if store_content(&config, &storage, &app_handle, &content, &hash) {
            state.settle();
            last_hash = hash;
        } else {
            state.failed(Instant::now());
        }
    }

    log::info!("tmux inotify 监听已停止");
}

fn rebuild_missing_hook() {
    if !crate::commands::tmux_bindings_present() {
        log::warn!("tmux copy-pipe 绑定丢失，正在重建...");
        if let Err(error) = crate::commands::setup_tmux_hook() {
            log::warn!("tmux 绑定重建失败: {}", error);
        }
    }
}

fn store_content(
    config: &Arc<Mutex<AppConfig>>,
    storage: &Arc<Mutex<StorageEngine>>,
    app_handle: &AppHandle,
    content: &str,
    hash: &str,
) -> bool {
    if content.len() > MAX_CLIPBOARD_TEXT_BYTES {
        log::warn!("拒绝超过 {MAX_CLIPBOARD_TEXT_BYTES} 字节的 tmux 缓冲内容");
        return true;
    }
    let max_history = config
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .max_history;
    let byte_size = content.len() as i64;
    let sensitive = is_sensitive_text(content);

    let result = {
        let storage = storage.lock().unwrap_or_else(|error| error.into_inner());
        match storage.insert_clip(
            &ContentType::Text,
            Some(content),
            None,
            None,
            hash,
            byte_size,
            sensitive,
        ) {
            Ok(clip) => {
                let removed = storage.cleanup_old_entries(max_history).ok();
                Some((clip, removed))
            }
            Err(error) => {
                log::warn!("tmux 缓冲区保存失败: {}", error);
                None
            }
        }
    };

    if let Some((clip, removed)) = result {
        if let Some(removed_ids) = removed {
            for removed_id in removed_ids {
                let _ = app_handle.emit("clip-removed", removed_id);
            }
        }
        let _ = app_handle.emit("clip-added", &clip.for_list());
        log::debug!("tmux 缓冲区内容（inotify），大小: {} 字节", byte_size);
        true
    } else {
        false
    }
}

fn read_bounded_content(path: &std::path::Path) -> std::io::Result<Option<String>> {
    let file = std::fs::File::open(path)?;
    let mut content = String::new();
    file.take((MAX_CLIPBOARD_TEXT_BYTES + 1) as u64)
        .read_to_string(&mut content)?;
    if content.len() > MAX_CLIPBOARD_TEXT_BYTES {
        return Ok(None);
    }
    Ok(Some(content))
}
