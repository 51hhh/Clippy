//! Windows / macOS 原生自动粘贴。
//!
//! 面板出现前先记录前台目标；选中条目、面板隐藏后恢复该目标，再注入系统对应的粘贴
//! 组合键。目标失效、焦点恢复失败或权限不足都返回结构化错误，由 command 层安全降级为
//! “内容已复制到剪贴板”。

use super::{PasteBackend, PasteError};
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};

fn with_input_backend<I>(
    initialize: impl FnOnce() -> Result<I, PasteError>,
    validate: impl FnOnce() -> Result<(), PasteError>,
    inject: impl FnOnce(I) -> Result<(), PasteError>,
) -> Result<(), PasteError> {
    let input = initialize()?;
    // 激活/等待和后端初始化以后再复核目标，失败时尚未发送任何按键。
    validate()?;
    inject(input)
}

fn inject_paste(
    modifier: Key,
    modifier_name: &str,
    validate: impl FnOnce() -> Result<(), PasteError>,
) -> Result<(), PasteError> {
    let injection = |action: &str| {
        let action = action.to_string();
        move |error: enigo::InputError| PasteError::KeyInjection {
            action,
            detail: error.to_string(),
        }
    };
    with_input_backend(
        || {
            Enigo::new(&Settings::default())
                .map_err(|error| PasteError::InputBackendUnavailable(error.to_string()))
        },
        validate,
        |mut enigo| {
            enigo
                .key(modifier, Press)
                .map_err(injection(&format!("按下 {modifier_name}")))?;
            let click = enigo.key(Key::Unicode('v'), Click);
            let release = enigo.key(modifier, Release);
            click.map_err(injection("按下 V"))?;
            release.map_err(injection(&format!("释放 {modifier_name}")))?;
            Ok(())
        },
    )
}

#[cfg(target_os = "windows")]
mod implementation {
    use super::*;
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    type Hwnd = *mut c_void;

    #[derive(Clone)]
    pub struct Target {
        window: usize,
        process_id: u32,
    }

    #[derive(Clone, Copy)]
    struct WindowSnapshot {
        exists: bool,
        process_id: u32,
        foreground: usize,
    }

    impl Target {
        fn validate_snapshot(&self, snapshot: WindowSnapshot) -> Result<(), PasteError> {
            if !snapshot.exists
                || snapshot.process_id == 0
                || snapshot.process_id != self.process_id
            {
                return Err(PasteError::NativeTargetInvalid);
            }
            if snapshot.foreground != self.window {
                return Err(PasteError::NativeFocusNotRestored(
                    "Foreground window changed before paste input".to_string(),
                ));
            }
            Ok(())
        }

        fn validate_before_input(&self) -> Result<(), PasteError> {
            let window = self.window as Hwnd;
            // SAFETY: Win32 查询接受失效句柄并返回失败；这里不解引用 HWND。
            let exists = unsafe { IsWindow(window) } != 0;
            let mut process_id = 0;
            // SAFETY: PID 输出指向有效栈变量；失败保持初始零值，由快照校验拒绝。
            unsafe { GetWindowThreadProcessId(window, &mut process_id) };
            let foreground = unsafe { GetForegroundWindow() } as usize;
            self.validate_snapshot(WindowSnapshot {
                exists,
                process_id,
                foreground,
            })
        }
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> Hwnd;
        fn GetWindowThreadProcessId(window: Hwnd, process_id: *mut u32) -> u32;
        fn IsWindow(window: Hwnd) -> i32;
        fn SetForegroundWindow(window: Hwnd) -> i32;
    }

    pub fn backend() -> PasteBackend {
        PasteBackend::WindowsSendInput
    }

    pub fn capture_target() -> Result<Target, PasteError> {
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return Err(PasteError::NativeTargetMissing);
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id == 0 || process_id == std::process::id() {
            return Err(PasteError::NativeTargetMissing);
        }
        Ok(Target {
            window: window as usize,
            process_id,
        })
    }

    pub fn permission_ready() -> bool {
        true
    }

    pub fn can_request_permission() -> bool {
        false
    }

    pub fn permission_detail() -> &'static str {
        "Windows input injection is unavailable"
    }

    pub fn request_permission() {}

    pub fn paste(target: Option<Target>) -> Result<(), PasteError> {
        let target = target.ok_or(PasteError::NativeTargetMissing)?;
        let window = target.window as Hwnd;
        if unsafe { IsWindow(window) } == 0 {
            return Err(PasteError::NativeTargetInvalid);
        }
        let mut current_process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut current_process_id) };
        if current_process_id == 0 || current_process_id != target.process_id {
            // HWND 可能在捕获后被销毁并复用，不能把按键发给新的窗口所有者。
            return Err(PasteError::NativeTargetInvalid);
        }

        crate::platform::ensure_windows_input_target_integrity(target.process_id).map_err(
            |error| match error {
                crate::platform::WindowsInputSecurityError::Query(detail) => {
                    PasteError::WindowsIntegrityQuery(detail)
                }
                crate::platform::WindowsInputSecurityError::IntegrityBoundary {
                    current_rid,
                    target_rid,
                } => PasteError::WindowsIntegrityBoundary {
                    current_rid,
                    target_rid,
                },
            },
        )?;

        if unsafe { SetForegroundWindow(window) } == 0 {
            return Err(PasteError::NativeFocusNotRestored(
                "SetForegroundWindow 被系统拒绝".to_string(),
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if unsafe { GetForegroundWindow() } == window {
                return inject_paste(Key::Control, "Control", || target.validate_before_input());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(PasteError::NativeFocusNotRestored(
            "前台窗口在 500ms 内未切回目标".to_string(),
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::cell::{Cell, RefCell};

        const TARGET: Target = Target {
            window: 100,
            process_id: 37,
        };
        const READY: WindowSnapshot = WindowSnapshot {
            exists: true,
            process_id: 37,
            foreground: 100,
        };

        fn attempt_after_initialization(
            snapshot: WindowSnapshot,
            injections: &Cell<usize>,
        ) -> Result<(), PasteError> {
            let state = Cell::new(READY);
            with_input_backend(
                || {
                    state.set(snapshot);
                    Ok(())
                },
                || TARGET.validate_snapshot(state.get()),
                |_| {
                    injections.set(injections.get() + 1);
                    Ok(())
                },
            )
        }

        #[test]
        fn destroyed_target_after_initialization_never_injects() {
            let injections = Cell::new(0);
            let result = attempt_after_initialization(
                WindowSnapshot {
                    exists: false,
                    ..READY
                },
                &injections,
            );
            assert_eq!(injections.get(), 0);
            assert!(matches!(result, Err(PasteError::NativeTargetInvalid)));
        }

        #[test]
        fn changed_or_unknown_owner_after_initialization_never_injects() {
            for process_id in [0, 71] {
                let injections = Cell::new(0);
                let result = attempt_after_initialization(
                    WindowSnapshot {
                        process_id,
                        ..READY
                    },
                    &injections,
                );
                assert_eq!(injections.get(), 0);
                assert!(matches!(result, Err(PasteError::NativeTargetInvalid)));
            }
        }

        #[test]
        fn changed_foreground_after_initialization_never_injects() {
            for foreground in [0, 101] {
                let injections = Cell::new(0);
                let result = attempt_after_initialization(
                    WindowSnapshot {
                        foreground,
                        ..READY
                    },
                    &injections,
                );
                assert_eq!(injections.get(), 0);
                assert!(matches!(result, Err(PasteError::NativeFocusNotRestored(_))));
            }
        }

        #[test]
        fn ready_target_initializes_then_validates_then_injects() {
            let events = RefCell::new(Vec::new());
            with_input_backend(
                || {
                    events.borrow_mut().push("initialize");
                    Ok(())
                },
                || {
                    events.borrow_mut().push("validate");
                    TARGET.validate_snapshot(READY)
                },
                |_| {
                    events.borrow_mut().push("inject");
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(&*events.borrow(), &["initialize", "validate", "inject"]);
        }

        #[test]
        fn unavailable_backend_never_validates_or_injects() {
            let validates = Cell::new(0);
            let injections = Cell::new(0);
            let result = with_input_backend(
                || {
                    Err::<(), _>(PasteError::InputBackendUnavailable(
                        "offline fixture".to_string(),
                    ))
                },
                || {
                    validates.set(1);
                    Ok(())
                },
                |_| {
                    injections.set(1);
                    Ok(())
                },
            );
            assert!(matches!(
                result,
                Err(PasteError::InputBackendUnavailable(_))
            ));
            assert_eq!((validates.get(), injections.get()), (0, 0));
        }

        #[test]
        fn injection_error_keeps_existing_classification() {
            let result = with_input_backend(
                || Ok(()),
                || TARGET.validate_snapshot(READY),
                |_| {
                    Err(PasteError::KeyInjection {
                        action: "fixture".to_string(),
                        detail: "offline failure".to_string(),
                    })
                },
            );
            assert!(matches!(result, Err(PasteError::KeyInjection { .. })));
        }
    }
}

#[cfg(target_os = "macos")]
mod implementation {
    use super::*;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
    use std::time::{Duration, Instant};

    pub type Target = i32;

    pub fn backend() -> PasteBackend {
        PasteBackend::MacosQuartz
    }

    pub fn capture_target() -> Result<Target, PasteError> {
        let application = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .ok_or(PasteError::NativeTargetMissing)?;
        let process_id = application.processIdentifier();
        if process_id <= 0 || process_id as u32 == std::process::id() {
            return Err(PasteError::NativeTargetMissing);
        }
        Ok(process_id)
    }

    pub fn permission_ready() -> bool {
        crate::platform::macos_accessibility_trusted()
    }

    pub fn can_request_permission() -> bool {
        true
    }

    pub fn permission_detail() -> &'static str {
        "macOS Accessibility permission is required to paste automatically"
    }

    pub fn request_permission() {
        crate::platform::request_macos_accessibility_permission();
    }

    pub fn paste(target: Option<Target>) -> Result<(), PasteError> {
        if !permission_ready() {
            return Err(PasteError::MacosAccessibilityPermissionRequired);
        }
        let process_id = target.ok_or(PasteError::NativeTargetMissing)?;
        let application = NSRunningApplication::runningApplicationWithProcessIdentifier(process_id)
            .ok_or(PasteError::NativeTargetInvalid)?;
        if !application.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows) {
            return Err(PasteError::NativeFocusNotRestored(
                "NSRunningApplication 拒绝激活目标应用".to_string(),
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if application.isActive() {
                return inject_paste(Key::Meta, "Command", || Ok(()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(PasteError::NativeFocusNotRestored(
            "前台应用在 500ms 内未切回目标".to_string(),
        ))
    }
}

pub(super) use implementation::*;
