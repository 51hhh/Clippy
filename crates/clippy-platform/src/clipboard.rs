use arboard::Error;
pub use arboard::ImageData;
use clippy_core::clipboard::{strip_html_tags, MAX_CLIPBOARD_TEXT_BYTES};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::sync::Arc;
use std::time::Duration;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as notifications;
#[cfg(target_os = "linux")]
use x11 as notifications;

#[derive(Clone)]
pub struct ClipboardWake {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    signal: Arc<notifications::RequestSignal>,
}

impl ClipboardWake {
    pub fn new() -> Option<Self> {
        #[cfg(target_os = "linux")]
        if crate::session::is_wayland() {
            return None;
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let signal = notifications::RequestSignal::new().ok()?;
            Some(Self {
                signal: Arc::new(signal),
            })
        }
        #[cfg(target_os = "macos")]
        None
    }

    pub fn notify(&self) {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.signal.notify();
    }
}

pub enum Content {
    Text(String),
    Html { html: String, text: String },
    Image(ImageData<'static>),
}

pub struct Clipboard {
    handle: Backend,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    notifications: Option<notifications::SelectionChanges>,
}

enum Backend {
    Native(arboard::Clipboard),
    #[cfg(target_os = "linux")]
    Portal(crate::paste::ClipboardClient),
}

impl Clipboard {
    pub fn open() -> Result<Self, Error> {
        Ok(Self {
            handle: Backend::Native(arboard::Clipboard::new()?),
            #[cfg(target_os = "linux")]
            notifications: (!crate::session::is_wayland())
                .then(x11::SelectionChanges::connect)
                .and_then(Result::ok),
            #[cfg(target_os = "windows")]
            notifications: windows::SelectionChanges::connect().ok(),
        })
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn with_portal(handle: crate::paste::ClipboardClient) -> Self {
        Self {
            handle: Backend::Portal(handle),
            notifications: None,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn portal_closed(&self) -> bool {
        matches!(&self.handle, Backend::Portal(handle) if handle.generation().is_none())
    }

    /// 返回 false 时由调用方按原间隔轮询。
    pub fn wait_for_activity(&mut self, wake: &ClipboardWake, timeout: Duration) -> bool {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let Some(changes) = &self.notifications else {
                return false;
            };
            if changes.wait(&wake.signal, timeout).is_ok() {
                return true;
            }
            self.notifications = None;
            false
        }
        #[cfg(target_os = "macos")]
        {
            let _ = (wake, timeout);
            false
        }
    }

    /// None 表示后端没有可靠的剪贴板代次，调用者仍可按内容短路。
    pub fn generation(&mut self) -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            if let Backend::Portal(handle) = &self.handle {
                return handle.generation();
            }
            let result = self.notifications.as_mut()?.generation();
            match result {
                Ok(value) => Some(value),
                Err(_) => {
                    self.notifications = None;
                    None
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
            let sequence = unsafe { GetClipboardSequenceNumber() };
            (sequence != 0).then_some(u64::from(sequence))
        }
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSPasteboard;
            Some(NSPasteboard::generalPasteboard().changeCount() as u64)
        }
    }

    pub fn read(&mut self) -> Result<Option<Content>, Error> {
        match &mut self.handle {
            #[cfg(target_os = "linux")]
            Backend::Portal(handle) => handle.read().map_err(portal_error),
            Backend::Native(handle) => {
                #[cfg(target_os = "windows")]
                let (html, text) = match read_flavor(handle.get().html_with_text())? {
                    Some((html, text)) if !html.is_empty() => (Some(html), text),
                    _ => (None, read_flavor(handle.get_text())?),
                };
                #[cfg(not(target_os = "windows"))]
                let (html, text) = (
                    read_flavor(handle.get().html())?,
                    read_flavor(handle.get_text())?,
                );
                let text = text
                    .filter(|value| !value.is_empty() && value.len() <= MAX_CLIPBOARD_TEXT_BYTES);
                let html = html.filter(|value| !value.is_empty());
                if let Some(html) = html.filter(|value| value.len() <= MAX_CLIPBOARD_TEXT_BYTES) {
                    return Ok(Some(Content::Html {
                        text: text.unwrap_or_else(|| strip_html_tags(&html)),
                        html,
                    }));
                }
                if let Some(text) = text {
                    return Ok(Some(Content::Text(text)));
                }
                Ok(read_flavor(handle.get_image())?.map(Content::Image))
            }
        }
    }

    pub fn write(&mut self, content: Content) -> Result<(), Error> {
        match &mut self.handle {
            #[cfg(target_os = "linux")]
            Backend::Portal(handle) => handle.write(content).map_err(portal_error),
            Backend::Native(handle) => match content {
                Content::Text(text) => handle.set_text(text),
                Content::Html { html, text } => handle.set_html(html, Some(text)),
                Content::Image(image) => handle.set_image(image),
            },
        }
    }

    pub fn write_image(&mut self, image: ImageData<'_>) -> Result<(), Error> {
        match &mut self.handle {
            Backend::Native(handle) => handle.set_image(image),
            #[cfg(target_os = "linux")]
            Backend::Portal(handle) => handle.write_image(image).map_err(portal_error),
        }
    }
}

#[cfg(target_os = "linux")]
fn portal_error(error: anyhow::Error) -> Error {
    Error::Unknown {
        description: format!("{error:#}"),
    }
}

fn read_flavor<T>(result: Result<T, Error>) -> Result<Option<T>, Error> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Error::ContentNotAvailable | Error::ConversionFailure) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes to the desktop clipboard; use an isolated Windows session"]
    fn windows_wait_wakes_on_clipboard_update_and_request() {
        use std::{thread, time::Instant};

        let mut reader = Clipboard::open().unwrap();
        let request = ClipboardWake::new().unwrap();
        let previous = reader.generation();
        let text = "Clippy 剪贴板通知 Ω";
        thread::scope(|scope| {
            let started = Instant::now();
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(40));
                Clipboard::open()
                    .unwrap()
                    .write(Content::Text(text.into()))
                    .unwrap();
            });
            loop {
                assert!(reader.wait_for_activity(&request, Duration::from_secs(2)));
                assert!(started.elapsed() < Duration::from_secs(1));
                if reader.generation() != previous {
                    break;
                }
            }
        });
        assert!(matches!(reader.read().unwrap(), Some(Content::Text(value)) if value == text));

        assert!(reader.wait_for_activity(&request, Duration::ZERO));
        let started = Instant::now();
        request.notify();
        assert!(reader.wait_for_activity(&request, Duration::from_secs(2)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    #[ignore = "writes to the desktop clipboard; use an isolated Windows session"]
    fn windows_reads_text_and_image_without_html() {
        let mut source = Clipboard::open().unwrap();
        let mut reader = Clipboard::open().unwrap();
        let text = "Clippy 剪贴板 Ω";
        source.write(Content::Text(text.into())).unwrap();
        assert!(matches!(reader.read().unwrap(), Some(Content::Text(value)) if value == text));

        let bytes = [12, 34, 56, 128];
        source
            .write_image(ImageData {
                width: 1,
                height: 1,
                bytes: std::borrow::Cow::Borrowed(&bytes),
            })
            .unwrap();
        assert!(matches!(reader.read().unwrap(), Some(Content::Image(image))
            if image.width == 1 && image.height == 1 && image.bytes.as_ref() == bytes));
    }

    #[test]
    #[ignore = "writes to the desktop clipboard; use an isolated Windows session"]
    fn windows_reads_text_with_empty_html() {
        let mut source = Clipboard::open().unwrap();
        let mut reader = Clipboard::open().unwrap();
        let text = "Clippy 剪贴板 Ω";
        source
            .write(Content::Html {
                html: String::new(),
                text: text.into(),
            })
            .unwrap();
        assert!(matches!(reader.read().unwrap(), Some(Content::Text(value)) if value == text));
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use rustix::{
        event::{eventfd, poll, EventfdFlags, PollFd, PollFlags, Timespec},
        fd::OwnedFd,
        io::{self, Errno},
    };
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::{
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::{ConnectionExt as _, CreateWindowAux, WindowClass},
        Event,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::COPY_DEPTH_FROM_PARENT;

    pub(super) struct RequestSignal(OwnedFd);

    impl RequestSignal {
        pub(super) fn new() -> io::Result<Self> {
            eventfd(0, EventfdFlags::NONBLOCK | EventfdFlags::CLOEXEC).map(Self)
        }

        pub(super) fn notify(&self) {
            match io::retry_on_intr(|| io::write(&self.0, &1_u64.to_ne_bytes())) {
                Ok(_) | Err(Errno::AGAIN) => (),
                Err(error) => panic!("唤醒剪贴板工作线程失败: {error}"),
            }
        }
    }

    pub(super) struct SelectionChanges {
        connection: RustConnection,
        generation: u64,
    }

    impl SelectionChanges {
        pub(super) fn connect() -> anyhow::Result<Self> {
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
            let selection = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
            connection
                .xfixes_select_selection_input(
                    window,
                    selection,
                    SelectionEventMask::SET_SELECTION_OWNER
                        | SelectionEventMask::SELECTION_WINDOW_DESTROY
                        | SelectionEventMask::SELECTION_CLIENT_CLOSE,
                )?
                .check()?;
            Ok(Self {
                connection,
                generation: 0,
            })
        }

        pub(super) fn wait(&self, signal: &RequestSignal, timeout: Duration) -> io::Result<()> {
            let timeout = Timespec::try_from(timeout).map_err(|_| Errno::INVAL)?;
            let mut ready = [
                PollFd::new(&signal.0, PollFlags::IN),
                PollFd::new(self.connection.stream(), PollFlags::IN),
            ];
            match poll(&mut ready, Some(&timeout)) {
                Ok(_) => (),
                Err(Errno::INTR) => return Ok(()),
                Err(error) => return Err(error),
            }
            if ready[0].revents().contains(PollFlags::IN) {
                let mut count = [0_u8; 8];
                io::retry_on_intr(|| io::read(&signal.0, &mut count[..]))?;
            }
            if ready[1]
                .revents()
                .intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL)
            {
                return Err(Errno::PIPE);
            }
            Ok(())
        }

        pub(super) fn generation(&mut self) -> anyhow::Result<u64> {
            while let Some(event) = self.connection.poll_for_event()? {
                if matches!(event, Event::XfixesSelectionNotify(_)) {
                    self.generation = self.generation.wrapping_add(1);
                }
            }
            Ok(self.generation)
        }
    }
}
