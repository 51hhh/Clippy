//! 用真实文件共享句柄控制已提交产物提升，不替换生产文件 I/O。

use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

pub(in crate::recording) struct FileHold {
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<bool>>,
}

fn open_held(path: &Path) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)?;
    assert!(super::windows_sharing::delete_open_has_sharing_violation(
        path
    ));
    Ok(file)
}

impl FileHold {
    pub(in crate::recording) fn after_manifest(
        path: &Path,
        directory: &Path,
        ready: impl Fn(&serde_json::Value) -> bool + Send + 'static,
    ) -> Self {
        let held = open_held(path).unwrap();
        Self::spawn(directory.to_path_buf(), move |_| Some(held), ready)
    }

    #[cfg(feature = "recording-vp9-prototype")]
    pub(in crate::recording) fn future_partial(
        path: &Path,
        directory: &Path,
        ready: impl Fn(&serde_json::Value) -> bool + Send + 'static,
    ) -> Self {
        let path = path.to_path_buf();
        Self::spawn(
            directory.to_path_buf(),
            move |stop| {
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    match open_held(&path) {
                        Ok(file) => return Some(file),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => panic!("打开真实恢复 partial 失败: {error}"),
                    }
                    if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                        return None;
                    }
                    thread::yield_now();
                }
            },
            ready,
        )
    }

    fn spawn(
        directory: PathBuf,
        acquire: impl FnOnce(&AtomicBool) -> Option<File> + Send + 'static,
        ready: impl Fn(&serde_json::Value) -> bool + Send + 'static,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        let (started, start) = mpsc::channel();
        let join = thread::spawn(move || {
            started.send(()).unwrap();
            let Some(held) = acquire(&stop) else {
                return false;
            };
            let deadline = Instant::now() + Duration::from_secs(30);
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                if fs::read(directory.join("manifest.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .is_some_and(|value| ready(&value))
                {
                    // 清单已提交后再短暂持有，保证红基线覆盖真正的原子提升边界。
                    thread::sleep(Duration::from_millis(75));
                    drop(held);
                    return true;
                }
                thread::sleep(Duration::from_millis(1));
            }
            drop(held);
            false
        });
        start.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            cancel,
            join: Some(join),
        }
    }

    pub(in crate::recording) fn release(mut self) -> bool {
        self.cancel.store(true, Ordering::Release);
        self.join.take().unwrap().join().unwrap()
    }
}

impl Drop for FileHold {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
