//! Windows 私有文件 ACL：只给当前登录用户完整控制，并阻断父目录的宽松 DACL 继承。

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
#[cfg(test)]
use std::ptr::addr_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, GENERIC_ALL, HANDLE};
#[cfg(test)]
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::{
    SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SET_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
#[cfg(test)]
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetSecurityDescriptorControl, IsValidAcl, ACCESS_ALLOWED_ACE, ACE_HEADER,
    SE_DACL_PROTECTED,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, IsValidSid, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
    PROTECTED_DACL_SECURITY_INFORMATION, PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
#[cfg(test)]
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` 由 OpenProcessToken 返回，且只在这里关闭一次。
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: SetEntriesInAclW/GetNamedSecurityInfoW 的输出按合同必须用 LocalFree 释放。
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

/// `GetTokenInformation(TokenUser)` 返回变长结构；用 `usize` 缓冲区保证 TOKEN_USER 对齐。
struct CurrentUser {
    words: Vec<usize>,
}

impl CurrentUser {
    fn query() -> io::Result<Self> {
        let mut raw_token = null_mut();
        // SAFETY: 输出句柄指向有效栈变量；OwnedHandle 接管成功返回的句柄。
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle(raw_token);

        let mut required = 0u32;
        // 第一次调用按 Win32 合同只查询所需长度，失败且写回非零长度是正常路径。
        unsafe {
            GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required);
        }
        if required < std::mem::size_of::<TOKEN_USER>() as u32 {
            return Err(io::Error::last_os_error());
        }

        let word_size = std::mem::size_of::<usize>();
        let mut words = vec![0usize; (required as usize).div_ceil(word_size)];
        // SAFETY: 缓冲区至少为 required 字节且具备 TOKEN_USER 所需对齐，token 在调用期间有效。
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                words.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }

        let user = Self { words };
        // SAFETY: 成功的 TokenUser 查询保证缓冲区以 TOKEN_USER 开头，SID 位于同一缓冲区内。
        if unsafe { IsValidSid(user.sid()) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "当前 Windows 用户 SID 无效",
            ));
        }
        Ok(user)
    }

    fn sid(&self) -> PSID {
        // SAFETY: `words` 由成功的 GetTokenInformation(TokenUser) 填充且保持存活、对齐。
        unsafe { (*(self.words.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows 路径包含 NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}

fn from_win32(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

pub(super) fn replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide_path(source)?;
    let destination = wide_path(destination)?;
    // SAFETY: 两个路径都是 NUL 结尾的 UTF-16，生命周期覆盖整个调用。
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn restrict(path: &Path, directory: bool) -> io::Result<()> {
    let path = wide_path(path)?;
    let user = CurrentUser::query()?;
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL,
        grfAccessMode: SET_ACCESS,
        grfInheritance: if directory {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            NO_INHERITANCE
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: user.sid().cast(),
        },
    };

    let mut acl: *mut ACL = null_mut();
    // SAFETY: entry 及其中的 SID 在调用期间有效；输出 ACL 交给 LocalAllocation。
    let result = unsafe { SetEntriesInAclW(1, &entry, null(), &mut acl) };
    if result != ERROR_SUCCESS {
        return Err(from_win32(result));
    }
    let _acl = LocalAllocation(acl.cast());

    // PROTECTED_DACL 阻断父目录以后重新传播 Users/Everyone 等宽松 ACE。
    // SAFETY: path 为 NUL 结尾；acl 是 SetEntriesInAclW 返回且在本调用期间有效的 DACL。
    let result = unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(from_win32(result))
    }
}

#[cfg(test)]
pub(super) fn is_private(path: &Path) -> bool {
    private_acl(path).unwrap_or(false)
}

#[cfg(test)]
fn private_acl(path: &Path) -> io::Result<bool> {
    let path = wide_path(path)?;
    let user = CurrentUser::query()?;
    let mut acl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: path 为 NUL 结尾，输出 descriptor 由 LocalFree 管理，acl 指向 descriptor 内部。
    let result = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(from_win32(result));
    }
    let _descriptor = LocalAllocation(descriptor);
    if descriptor.is_null() || acl.is_null() {
        // NULL DACL 代表任何人完全访问，绝不能当成“没有权限”。
        return Ok(false);
    }

    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: descriptor 来自成功的 GetNamedSecurityInfoW，两个输出指针有效。
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
        || control & SE_DACL_PROTECTED == 0
    {
        return Ok(false);
    }
    // SAFETY: acl 位于仍存活的 descriptor 内。空 DACL 会拒绝所有人，但它也会让当前用户
    // 无法继续维护文件，不符合“仅当前用户可读写”的契约。
    if unsafe { IsValidAcl(acl) } == 0 || unsafe { (*acl).AceCount } == 0 {
        return Ok(false);
    }

    // 目录的可继承权限可能被 Windows 规范化成多个 ACE。数量不是安全边界；真正的不变量是
    // 每一条都只能是授予当前用户的 allow ACE，不能夹入 Everyone、Users 或 deny 条目。
    let ace_count = unsafe { (*acl).AceCount };
    for index in 0..u32::from(ace_count) {
        let mut raw_ace = null_mut();
        // SAFETY: ACL 已验证，index 严格小于 AceCount。
        if unsafe { GetAce(acl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
            return Ok(false);
        }
        // SAFETY: GetAce 返回的每种 ACE 都以 ACE_HEADER 开头；确认类型后再按
        // ACCESS_ALLOWED_ACE 解释其余字段。
        let header = unsafe { &*raw_ace.cast::<ACE_HEADER>() };
        if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE {
            return Ok(false);
        }
        let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = addr_of!(ace.SidStart).cast_mut().cast();
        // SAFETY: ACCESS_ALLOWED_ACE 的 SidStart 是变长 SID 的首字段，user.sid() 在缓冲区内有效。
        if unsafe { IsValidSid(sid) == 0 || EqualSid(sid, user.sid()) == 0 } {
            return Ok(false);
        }
    }
    Ok(true)
}
