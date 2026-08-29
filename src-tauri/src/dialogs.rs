//! 系统文件对话框。集中封装 tauri-plugin-dialog，让初始目录、建议文件名和
//! PNG 过滤器在各处保持一致；Linux 下 GTK/Portal 的差异由插件负责。
//!
//! 这里的函数都会阻塞到用户操作完，调用方必须先切到阻塞线程。

use crate::image_io::SaveTarget;
use std::path::{Path, PathBuf};
use tauri_plugin_dialog::{DialogExt, FilePath};

/// 另存为对话框。返回 None 表示用户取消。
pub fn choose_png_save_path(app_handle: &tauri::AppHandle, target: &SaveTarget) -> Option<PathBuf> {
    let suggestion = crate::image_io::suggested_filename(target, "clippy-screenshot");
    app_handle
        .dialog()
        .file()
        .set_directory(prepared_directory(&target.directory))
        .set_file_name(suggestion)
        .add_filter("PNG Image", &["png"])
        .blocking_save_file()
        .and_then(local_path)
}

/// 选择保存目录，供设置页填写截图目录。返回 None 表示用户取消。
pub fn choose_directory(app_handle: &tauri::AppHandle, start: &Path) -> Option<PathBuf> {
    app_handle
        .dialog()
        .file()
        .set_directory(prepared_directory(start))
        .blocking_pick_folder()
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
