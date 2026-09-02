//! 系统文件对话框。集中封装 tauri-plugin-dialog，让初始目录在各处保持一致；
//! Linux 下 GTK/Portal 的差异由插件负责。
//!
//! 这里的函数都会阻塞到用户操作完，调用方必须先切到阻塞线程。

use std::path::{Path, PathBuf};
use tauri_plugin_dialog::{DialogExt, FilePath};

/// 选择保存目录，供设置页填写截图目录。返回 None 表示用户取消。
pub fn choose_directory(app_handle: &tauri::AppHandle, start: &Path) -> Option<PathBuf> {
    app_handle
        .dialog()
        .file()
        .set_directory(prepared_directory(start))
        .blocking_pick_folder()
        .and_then(local_path)
}

/// 选择一张本地 PNG。取消或 Portal 返回非本地 URI 时返回 `None`。
pub fn choose_png(app_handle: &tauri::AppHandle, start: &Path) -> Option<PathBuf> {
    let start = if start.is_dir() {
        start.to_path_buf()
    } else {
        crate::image_io::expand_user_path("~")
    };
    app_handle
        .dialog()
        .file()
        .set_directory(start)
        .add_filter("PNG image", &["png"])
        .blocking_pick_file()
        .and_then(local_path)
}

/// 初始目录不存在时对话框会退回到主目录，先建出来让用户看到预期位置。
fn prepared_directory(directory: &Path) -> PathBuf {
    if let Err(error) = std::fs::create_dir_all(directory) {
        log::warn!("创建对话框初始目录失败，改用主目录: {error}");
        return crate::image_io::expand_user_path("~");
    }
    directory.to_path_buf()
}

/// 桌面端选出的一定是真实路径；拿不到路径按取消处理，不猜一个位置去写。
fn local_path(selected: FilePath) -> Option<PathBuf> {
    match selected.into_path() {
        Ok(path) => Some(path),
        Err(error) => {
            log::warn!("对话框返回的位置不是本地路径: {error}");
            None
        }
    }
}
