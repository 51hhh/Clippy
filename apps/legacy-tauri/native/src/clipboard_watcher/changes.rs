//! 只用可靠的 selection 变更通知跳过读取；缺扩展、断连和其他平台都退回轮询。
pub(super) struct ChangeMonitor {
    #[cfg(target_os = "linux")]
    x11: Option<X11Selection>,
    first: bool,
}

impl ChangeMonitor {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            x11: X11Selection::connect()
                .map_err(|error| log::debug!("剪贴板变更通知不可用，回退轮询: {error}"))
                .ok(),
            first: true,
        }
    }

    /// 返回真正的新代次。回退平台每次返回 true，但不能将其误认为内容已改变。
    pub fn poll(&mut self) -> (bool, bool) {
        let first = std::mem::take(&mut self.first);
        #[cfg(target_os = "linux")]
        if let Some(x11) = &self.x11 {
            match x11.changed() {
                Ok(changed) => return (first || changed, true),
                Err(error) => {
                    log::warn!("剪贴板通知连接断开，回退轮询: {error}");
                    self.x11 = None;
                }
            }
        }
        let _ = first;
        (true, false)
    }
}

#[cfg(target_os = "linux")]
struct X11Selection {
    connection: x11rb::rust_connection::RustConnection,
}

#[cfg(target_os = "linux")]
impl X11Selection {
    fn connect() -> Result<Self, Box<dyn std::error::Error>> {
        use x11rb::{
            connection::Connection,
            protocol::{
                xfixes::{ConnectionExt as _, SelectionEventMask},
                xproto::{ConnectionExt as _, CreateWindowAux, WindowClass},
            },
            COPY_DEPTH_FROM_PARENT,
        };
        let (connection, screen) = x11rb::connect(None)?;
        connection.xfixes_query_version(5, 0)?.reply()?;
        let root = connection.setup().roots[screen].root;
        let window = connection.generate_id()?;
        connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )?
            .check()?;
        let clipboard = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        connection
            .xfixes_select_selection_input(
                window,
                clipboard,
                SelectionEventMask::SET_SELECTION_OWNER
                    | SelectionEventMask::SELECTION_WINDOW_DESTROY
                    | SelectionEventMask::SELECTION_CLIENT_CLOSE,
            )?
            .check()?;
        connection.flush()?;
        Ok(Self { connection })
    }

    fn changed(&self) -> Result<bool, Box<dyn std::error::Error>> {
        use x11rb::{
            connection::Connection,
            protocol::{xproto::ConnectionExt as _, Event},
        };
        // 这里只排空本通知连接上已生成的事件。应用写入的完成屏障由 arboard
        // 本地补丁在实际 set_selection_owner 连接执行；跨连接 roundtrip 不能替代它。
        self.connection.get_input_focus()?.reply()?;
        let mut changed = false;
        while let Some(event) = self.connection.poll_for_event()? {
            if matches!(event, Event::XfixesSelectionNotify(_)) {
                changed = true;
            }
        }
        Ok(changed)
    }
}
