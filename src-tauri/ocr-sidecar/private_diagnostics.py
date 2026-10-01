"""质量工具的全新私有诊断目录；不依赖第三方包或外部权限命令。"""

from __future__ import annotations

from contextlib import contextmanager
import ctypes
from ctypes import wintypes
from pathlib import Path
import sys


def _windows_api():
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    security = ctypes.WinDLL("advapi32", use_last_error=True)
    pointer = ctypes.c_void_p

    class SecurityAttributes(ctypes.Structure):
        _fields_ = [
            ("length", wintypes.DWORD),
            ("descriptor", pointer),
            ("inherit_handle", wintypes.BOOL),
        ]

    class TokenUser(ctypes.Structure):
        _fields_ = [("sid", pointer), ("attributes", wintypes.DWORD)]

    signatures = [
        (kernel.GetCurrentProcess, [], wintypes.HANDLE),
        (kernel.CloseHandle, [wintypes.HANDLE], wintypes.BOOL),
        (kernel.LocalFree, [pointer], pointer),
        (kernel.CreateDirectoryW, [wintypes.LPCWSTR, ctypes.POINTER(SecurityAttributes)], wintypes.BOOL),
        (
            security.OpenProcessToken,
            [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE)],
            wintypes.BOOL,
        ),
        (
            security.GetTokenInformation,
            [wintypes.HANDLE, ctypes.c_int, pointer, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)],
            wintypes.BOOL,
        ),
        (security.ConvertSidToStringSidW, [pointer, ctypes.POINTER(wintypes.LPWSTR)], wintypes.BOOL),
        (
            security.ConvertStringSecurityDescriptorToSecurityDescriptorW,
            [wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(pointer), ctypes.POINTER(wintypes.DWORD)],
            wintypes.BOOL,
        ),
        (security.GetSecurityDescriptorControl, [pointer, ctypes.POINTER(wintypes.WORD), ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
        (security.GetSecurityDescriptorDacl, [pointer, ctypes.POINTER(wintypes.BOOL), ctypes.POINTER(pointer), ctypes.POINTER(wintypes.BOOL)], wintypes.BOOL),
        (security.GetAclInformation, [pointer, pointer, wintypes.DWORD, ctypes.c_int], wintypes.BOOL),
        (security.GetAce, [pointer, wintypes.DWORD, ctypes.POINTER(pointer)], wintypes.BOOL),
        (
            security.GetNamedSecurityInfoW,
            [wintypes.LPCWSTR, ctypes.c_int, wintypes.DWORD, pointer, pointer, pointer, pointer, ctypes.POINTER(pointer)],
            wintypes.DWORD,
        ),
    ]
    for function, arguments, result in signatures:
        function.argtypes = arguments
        function.restype = result
    return kernel, security, SecurityAttributes, TokenUser


def _current_user_sid(api) -> str:
    kernel, security, _, token_type = api
    token = wintypes.HANDLE()
    if not security.OpenProcessToken(kernel.GetCurrentProcess(), 0x0008, ctypes.byref(token)):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        required = wintypes.DWORD()
        security.GetTokenInformation(token, 1, None, 0, ctypes.byref(required))
        if required.value < ctypes.sizeof(token_type):
            raise OSError("TokenUser 未返回有效长度")
        # 使用对齐的机器字数组，保持 SID 与 TokenUser 缓冲区同时存活。
        word_size = ctypes.sizeof(ctypes.c_size_t)
        buffer = (ctypes.c_size_t * ((required.value + word_size - 1) // word_size))()
        if not security.GetTokenInformation(token, 1, buffer, required.value, ctypes.byref(required)):
            raise ctypes.WinError(ctypes.get_last_error())
        user = ctypes.cast(buffer, ctypes.POINTER(token_type)).contents
        text = wintypes.LPWSTR()
        if not security.ConvertSidToStringSidW(user.sid, ctypes.byref(text)):
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            return text.value
        finally:
            kernel.LocalFree(ctypes.cast(text, ctypes.c_void_p))
    finally:
        kernel.CloseHandle(token)


@contextmanager
def _windows_descriptor(sddl: str, api):
    kernel, security, _, _ = api
    descriptor = ctypes.c_void_p()
    if not security.ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl, 1, ctypes.byref(descriptor), None
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        yield descriptor
    finally:
        kernel.LocalFree(descriptor)


def _descriptor_dacl(descriptor, api):
    _, security, _, _ = api
    control = wintypes.WORD()
    revision = wintypes.DWORD()
    if not security.GetSecurityDescriptorControl(descriptor, ctypes.byref(control), ctypes.byref(revision)):
        raise ctypes.WinError(ctypes.get_last_error())
    present, defaulted = wintypes.BOOL(), wintypes.BOOL()
    acl = ctypes.c_void_p()
    if not security.GetSecurityDescriptorDacl(
        descriptor, ctypes.byref(present), ctypes.byref(acl), ctypes.byref(defaulted)
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    # NULL DACL 与缺失 DACL 都不能视为空 ACL；保留状态供严格比较。
    state = (bool(present.value), bool(defaulted.value), bool(control.value & 0x1000))
    if not present.value or not acl.value:
        return state, None

    class AclSizeInformation(ctypes.Structure):
        _fields_ = [("ace_count", wintypes.DWORD), ("bytes_in_use", wintypes.DWORD), ("bytes_free", wintypes.DWORD)]

    class AceHeader(ctypes.Structure):
        _fields_ = [("type", wintypes.BYTE), ("flags", wintypes.BYTE), ("size", wintypes.WORD)]

    information = AclSizeInformation()
    if not security.GetAclInformation(acl, ctypes.byref(information), ctypes.sizeof(information), 2):
        raise ctypes.WinError(ctypes.get_last_error())
    entries = []
    for index in range(information.ace_count):
        ace = ctypes.c_void_p()
        if not security.GetAce(acl, index, ctypes.byref(ace)):
            raise ctypes.WinError(ctypes.get_last_error())
        header = ctypes.cast(ace, ctypes.POINTER(AceHeader)).contents
        entries.append(ctypes.string_at(ace, header.size))
    # 比较 ACE 原始字节：类型、继承标志、访问掩码和二进制 SID 均必须一致。
    # SDDL SID 别名与 AI 状态不改变这些权限；保护位 P 则必须保持一致。
    return state, tuple(entries)


def _expected_windows_dacl(sddl: str, api=None):
    api = api or _windows_api()
    with _windows_descriptor(sddl, api) as descriptor:
        return _descriptor_dacl(descriptor, api)


def _read_windows_dacl(path: Path, api=None):
    api = api or _windows_api()
    kernel, security, _, _ = api
    descriptor = ctypes.c_void_p()
    result = security.GetNamedSecurityInfoW(
        str(path), 1, 4, None, None, None, None, ctypes.byref(descriptor)
    )
    if result:
        raise ctypes.WinError(result)
    try:
        return _descriptor_dacl(descriptor, api)
    finally:
        kernel.LocalFree(descriptor)


def _create_windows_directory(path: Path) -> None:
    api = _windows_api()
    kernel, security, attributes_type, _ = api
    sid = _current_user_sid(api)
    # P 阻断父目录继承；OI/CI 让今后的诊断文件与子目录只继承当前用户 ACE。
    sddl = f"D:P(A;OICI;FA;;;{sid})"
    with _windows_descriptor(sddl, api) as descriptor:
        expected = _descriptor_dacl(descriptor, api)
        attributes = attributes_type(ctypes.sizeof(attributes_type), descriptor, False)
        # 在创建时应用 DACL，不先创建宽松目录再 chmod；已存在路径绝不覆盖或改权限。
        if not kernel.CreateDirectoryW(str(path), ctypes.byref(attributes)):
            raise ctypes.WinError(ctypes.get_last_error())
    try:
        if _read_windows_dacl(path, api) != expected:
            raise OSError("文件系统未保留私有诊断目录 DACL")
    except OSError:
        # 只移除本次刚创建、尚未写任何数据的空目录；失败时不退回宽松输出。
        path.rmdir()
        raise


def create_private_directory(path: Path) -> Path:
    # Win32 宽字符串遇 NUL 会截断；必须在创建前拒绝，避免创建另一个路径。
    if "\0" in str(path):
        raise ValueError("诊断目录路径包含 NUL")
    path = path.absolute()
    if sys.platform == "win32":
        _create_windows_directory(path)
    else:
        path.mkdir(mode=0o700, parents=False, exist_ok=False)
    return path.resolve()
