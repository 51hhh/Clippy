//! Windows 输入注入前的完整性边界检查。
//!
//! `SendInput` 遇到 UIPI 时不会报告可区分的错误，因此自动粘贴与长截图滚轮必须在注入前
//! 共用同一套进程令牌检查，不能各自猜测结果。

use std::io;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, IsValidSid,
    TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum WindowsInputSecurityError {
    #[error("无法查询 Windows 进程完整性级别: {0}")]
    Query(String),
    #[error("目标窗口完整性级别高于 Clippy（current={current_rid:#x}, target={target_rid:#x}）")]
    IntegrityBoundary { current_rid: u32, target_rid: u32 },
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 句柄由 OpenProcess/OpenProcessToken 返回，只在这里关闭一次。
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// 读取进程访问令牌的 Mandatory Integrity Level RID。
fn process_integrity_rid(process: HANDLE) -> Result<u32, WindowsInputSecurityError> {
    let mut raw_token = null_mut();
    // SAFETY: process 是当前进程伪句柄或 OwnedHandle 管理的有效进程句柄；输出写入栈变量。
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw_token) } == 0 {
        return Err(WindowsInputSecurityError::Query(
            io::Error::last_os_error().to_string(),
        ));
    }
    let token = OwnedHandle(raw_token);

    let mut required = 0u32;
    // 第一次调用按 Win32 合同只查询所需长度，失败且返回非零长度是正常路径。
    unsafe {
        GetTokenInformation(token.0, TokenIntegrityLevel, null_mut(), 0, &mut required);
    }
    if required < std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
        return Err(WindowsInputSecurityError::Query(
            "TokenIntegrityLevel 未返回有效缓冲区长度".to_string(),
        ));
    }

    // usize 缓冲区确保 TOKEN_MANDATORY_LABEL 具备正确对齐。
    let word_size = std::mem::size_of::<usize>();
    let mut words = vec![0usize; (required as usize).div_ceil(word_size)];
    // SAFETY: 缓冲区至少为 required 字节且正确对齐，token 在调用期间有效。
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            words.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(WindowsInputSecurityError::Query(
            io::Error::last_os_error().to_string(),
        ));
    }

    // SAFETY: 成功的 TokenIntegrityLevel 查询保证缓冲区以 TOKEN_MANDATORY_LABEL 开头。
    let sid = unsafe {
        (*(words.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()))
            .Label
            .Sid
    };
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(WindowsInputSecurityError::Query(
            "TokenIntegrityLevel 返回了无效 SID".to_string(),
        ));
    }

    // 完整性 RID 是 Mandatory Label SID 的最后一个 sub-authority。
    let count = unsafe { GetSidSubAuthorityCount(sid) };
    if count.is_null() || unsafe { *count } == 0 {
        return Err(WindowsInputSecurityError::Query(
            "Mandatory Label SID 缺少 sub-authority".to_string(),
        ));
    }
    let index = u32::from(unsafe { *count } - 1);
    let rid = unsafe { GetSidSubAuthority(sid, index) };
    if rid.is_null() {
        return Err(WindowsInputSecurityError::Query(
            "无法读取 Mandatory Label RID".to_string(),
        ));
    }
    Ok(unsafe { *rid })
}

fn ensure_integrity_order(
    current_rid: u32,
    target_rid: u32,
) -> Result<(), WindowsInputSecurityError> {
    if target_rid > current_rid {
        Err(WindowsInputSecurityError::IntegrityBoundary {
            current_rid,
            target_rid,
        })
    } else {
        Ok(())
    }
}

pub(crate) fn ensure_input_target_integrity(
    process_id: u32,
) -> Result<(), WindowsInputSecurityError> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(WindowsInputSecurityError::Query(format!(
            "无法打开目标进程: {}",
            io::Error::last_os_error()
        )));
    }
    let process = OwnedHandle(process);
    let current_rid = process_integrity_rid(unsafe { GetCurrentProcess() })?;
    let target_rid = process_integrity_rid(process.0)?;
    ensure_integrity_order(current_rid, target_rid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_only_higher_integrity_targets() {
        assert!(ensure_integrity_order(0x2000, 0x1000).is_ok());
        assert!(ensure_integrity_order(0x2000, 0x2000).is_ok());
        assert_eq!(
            ensure_integrity_order(0x2000, 0x3000).unwrap_err(),
            WindowsInputSecurityError::IntegrityBoundary {
                current_rid: 0x2000,
                target_rid: 0x3000,
            }
        );
    }
}
