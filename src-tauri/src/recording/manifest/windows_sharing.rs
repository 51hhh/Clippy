//! 仅录屏清单与已提交产物提升容忍短暂共享冲突，保留原权限与错误合同。

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};

const RETRY_BUDGET: Duration = Duration::from_millis(500);
const RETRY_INTERVAL: Duration = Duration::from_millis(25);
const MAX_ATTEMPTS: usize = 21;

pub(super) fn replace(source: &Path, destination: &Path) -> io::Result<()> {
    retry(
        || super::replace_private_file(source, destination),
        || source.exists(),
        || {
            delete_open_has_sharing_violation(source)
                || delete_open_has_sharing_violation(destination)
        },
    )
}

pub(super) fn retry(
    mut operation: impl FnMut() -> io::Result<()>,
    mut source_exists: impl FnMut() -> bool,
    mut confirmed_sharing: impl FnMut() -> bool,
) -> io::Result<()> {
    let deadline = Instant::now() + RETRY_BUDGET;
    let mut attempts = 0;
    loop {
        attempts += 1;
        let error = match operation() {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        let code = error.raw_os_error();
        if attempts >= MAX_ATTEMPTS
            || Instant::now() >= deadline
            || !matches!(code, Some(5 | 32 | 33))
            || !source_exists()
            || (code == Some(5) && !confirmed_sharing())
        {
            return Err(error);
        }
        std::thread::sleep(RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        // 不在预算已经耗尽后再发起一次调用；原生单次调用本身仍沿用原阻塞合同。
        if Instant::now() >= deadline {
            return Err(error);
        }
    }
}

pub(super) fn delete_open_has_sharing_violation(path: &Path) -> bool {
    let mut path: Vec<u16> = path.as_os_str().encode_wide().collect();
    if path.contains(&0) {
        return false;
    }
    path.push(0);
    // SAFETY: 路径为有效 NUL 结尾 UTF-16。只打开现有对象，不创建/删除，也不跟随 reparse；
    // 全部共享避免本探针阻塞其它读写。没有 DELETE_ON_CLOSE，成功句柄只在下方关闭一次。
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return io::Error::last_os_error().raw_os_error() == Some(32);
    }
    // SAFETY: CreateFileW 成功返回的句柄由本函数独占，未向任何其它线程或 API 转移。
    unsafe { CloseHandle(handle) };
    false
}
