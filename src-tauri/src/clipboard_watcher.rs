use crate::models::{AppConfig, ContentType};
use crate::storage::StorageEngine;
use arboard::Clipboard;
use sha2::{Digest, Sha256};
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
        match self.skip_hash.lock() {
            Ok(mut skip) => *skip = Some(hash),
            Err(e) => log::error!("剪贴板跳过哈希锁定失败: {}", e),
        }
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
            let mut r = match running.lock() {
                Ok(r) => r,
                Err(e) => {
                    log::error!("剪贴板监听器状态锁定失败: {}", e);
                    return;
                }
            };
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
            log::info!("剪贴板监听器已启动");

            loop {
                {
                    let r = match running.lock() {
                        Ok(r) => r,
                        Err(e) => {
                            log::error!("剪贴板监听器状态锁定失败: {}", e);
                            break;
                        }
                    };
                    if !*r {
                        break;
                    }
                }

                if let Ok(text) = clipboard.get_text() {
                    if !text.is_empty() {
                        let hash = compute_hash(text.as_bytes());
                        if hash != last_hash {
                            last_hash = hash.clone();

                            // Fix #1: 跳过 select_clip 写入的内容
                            {
                                let mut skip = match skip_hash.lock() {
                                    Ok(skip) => skip,
                                    Err(e) => {
                                        log::error!("剪贴板跳过哈希锁定失败: {}", e);
                                        break;
                                    }
                                };
                                if skip.as_deref() == Some(&hash) {
                                    *skip = None;
                                    continue;
                                }
                            }

                            // Fix #4: 先读取 config，再锁 storage，缩小锁范围
                            let max_history = match config.lock() {
                                Ok(config) => config.max_history,
                                Err(e) => {
                                    log::error!("配置锁定失败: {}", e);
                                    break;
                                }
                            };
                            let byte_size = text.len() as i64;

                            let result = {
                                let storage = match storage.lock() {
                                    Ok(storage) => storage,
                                    Err(e) => {
                                        log::error!("存储锁定失败: {}", e);
                                        break;
                                    }
                                };
                                let clip_result = storage.insert_clip(
                                    &ContentType::Text,
                                    Some(&text),
                                    None,
                                    None,
                                    &hash,
                                    byte_size,
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
