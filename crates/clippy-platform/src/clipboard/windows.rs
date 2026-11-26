use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::{null, null_mut},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HWND, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener},
        Threading::{CreateEventW, SetEvent},
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, MsgWaitForMultipleObjectsEx,
        PeekMessageW, HWND_MESSAGE, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_POSTMESSAGE,
        WM_CLIPBOARDUPDATE,
    },
};

pub(super) struct RequestSignal(OwnedHandle);

impl RequestSignal {
    pub(super) fn new() -> io::Result<Self> {
        let handle = unsafe { CreateEventW(null(), 0, 0, null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    pub(super) fn notify(&self) {
        let signalled = unsafe { SetEvent(self.0.as_raw_handle()) };
        assert_ne!(signalled, 0, "剪贴板工作线程的唤醒句柄有效");
    }
}

pub(super) struct SelectionChanges(HWND);

impl SelectionChanges {
    pub(super) fn connect() -> io::Result<Self> {
        // 系统直接向注册的窗口投递通知。仅消息窗口由等待它的剪贴板线程拥有。
        let window = unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            )
        };
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        if unsafe { AddClipboardFormatListener(window) } == 0 {
            let error = io::Error::last_os_error();
            unsafe { DestroyWindow(window) };
            return Err(error);
        }
        Ok(Self(window))
    }

    pub(super) fn wait(&self, signal: &RequestSignal, timeout: Duration) -> io::Result<()> {
        let handle = signal.0.as_raw_handle();
        let millis = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(1, &handle, millis, QS_POSTMESSAGE, MWMO_INPUTAVAILABLE)
        };
        match result {
            WAIT_OBJECT_0 | WAIT_TIMEOUT => Ok(()),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            result if result == WAIT_OBJECT_0 + 1 => {
                // 批量清掉已投递的通知，再由调用方检查最新代次和待办请求。
                let mut message = MSG::default();
                for _ in 0..32 {
                    if unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } == 0 {
                        break;
                    }
                    if message.hwnd != self.0 || message.message != WM_CLIPBOARDUPDATE {
                        unsafe { DispatchMessageW(&message) };
                    }
                }
                Ok(())
            }
            _ => unreachable!("剪贴板仅等待一个事件和线程消息"),
        }
    }
}

impl Drop for SelectionChanges {
    fn drop(&mut self) {
        unsafe {
            RemoveClipboardFormatListener(self.0);
            DestroyWindow(self.0);
        }
    }
}
