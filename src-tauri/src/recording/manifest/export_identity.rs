//! Windows 导出先核对实际句柄身份，文本路径不同仍可能是同一内部文件。

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
};

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    volume_serial_number: u64,
    file_id: [u8; 16],
}

pub(super) fn ensure_different_file(source: &File, destination: &Path) -> Result<(), String> {
    // 只请求元数据查询，不要求读取已有目标的内容；也不能用 create/truncate 打开目标。
    let destination = match OpenOptions::new().access_mode(0).open(destination) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("打开导出目标身份失败: {error}")),
    };
    let source_identity = read_identity(source.as_raw_handle())
        .map_err(|error| format!("读取录屏源文件身份失败: {error}"))?;
    let destination_identity = read_identity(destination.as_raw_handle())
        .map_err(|error| format!("读取导出目标身份失败: {error}"))?;
    if source_identity == destination_identity {
        return Err("不能用导出文件覆盖内部恢复产物".to_string());
    }
    Ok(())
}

fn read_identity(handle: HANDLE) -> io::Result<FileIdentity> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: 生产调用持有对应 File，句柄在查询期间有效；缓冲区完整容纳 FileIdInfo。
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            std::ptr::from_mut(&mut info).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(FileIdentity {
        volume_serial_number: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::{ERROR_INVALID_HANDLE, INVALID_HANDLE_VALUE};

    #[test]
    fn native_invalid_identity_handle_retains_os_error() {
        // Windows 对无效句柄返回错误；不构造已关闭的 File，也不重复关闭真实句柄。
        let error = read_identity(INVALID_HANDLE_VALUE).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_INVALID_HANDLE as i32));
    }
}
