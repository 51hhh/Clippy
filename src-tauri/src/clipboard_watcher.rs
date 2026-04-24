#![allow(dead_code)]
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
}

impl ClipboardWatcher {
    pub fn new() -> Self {
        Self {
            running: Arc::new(Mutex::new(false)),
        }
    }

    pub fn start(
        &self,
        app_handle: AppHandle,
        storage: Arc<Mutex<StorageEngine>>,
        config: Arc<Mutex<AppConfig>>,
    ) {
        let running = Arc::clone(&self.running);
        {
            let mut r = running.lock().unwrap();
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
                    let r = running.lock().unwrap();
                    if !*r {
                        break;
                    }
                }

                if let Ok(text) = clipboard.get_text() {
                    if !text.is_empty() {
                        let hash = compute_hash(text.as_bytes());
                        if hash != last_hash {
                            last_hash = hash.clone();
                            let byte_size = text.len() as i64;
                            let storage = storage.lock().unwrap();
                            match storage.insert_clip(
                                &ContentType::Text,
                                Some(&text),
                                None,
                                None,
                                &hash,
                                byte_size,
                            ) {
                                Ok(clip) => {
                                    let max_history = config.lock().unwrap().max_history;
                                    if let Ok(removed_ids) =
                                        storage.cleanup_old_entries(max_history)
                                    {
                                        for rid in removed_ids {
                                            let _ = app_handle.emit("clip-removed", rid);
                                        }
                                    }
                                    let _ = app_handle.emit("clip-added", &clip);
                                    log::debug!(
                                        "新剪贴板内容，类型: text, 大小: {} 字节",
                                        byte_size
                                    );
                                }
                                Err(e) => log::warn!("剪贴板内容保存失败: {}", e),
                            }
                        }
                    }
                }

                thread::sleep(Duration::from_millis(500));
            }
            log::info!("剪贴板监听器已停止");
        });
    }

    pub fn stop(&self) {
        let mut r = self.running.lock().unwrap();
        *r = false;
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
