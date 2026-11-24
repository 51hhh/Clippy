use arboard::Error;
pub use arboard::ImageData;
use clippy_core::clipboard::{strip_html_tags, MAX_CLIPBOARD_TEXT_BYTES};

pub enum Content {
    Text(String),
    Html { html: String, text: String },
    Image(ImageData<'static>),
}

pub struct Clipboard {
    handle: arboard::Clipboard,
    #[cfg(target_os = "linux")]
    notifications: Option<x11::SelectionChanges>,
}

impl Clipboard {
    pub fn open() -> Result<Self, Error> {
        Ok(Self {
            handle: arboard::Clipboard::new()?,
            #[cfg(target_os = "linux")]
            notifications: (!crate::session::is_wayland())
                .then(x11::SelectionChanges::connect)
                .and_then(Result::ok),
        })
    }

    /// None 表示后端没有可靠的剪贴板代次，调用者仍可按内容短路。
    pub fn generation(&mut self) -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
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
        #[cfg(target_os = "windows")]
        let (html, text) = match read_flavor(self.handle.get().html_with_text())? {
            Some((html, text)) => (Some(html), text),
            None => (None, read_flavor(self.handle.get_text())?),
        };
        #[cfg(not(target_os = "windows"))]
        let (html, text) = (
            read_flavor(self.handle.get().html())?,
            read_flavor(self.handle.get_text())?,
        );
        let text =
            text.filter(|value| !value.is_empty() && value.len() <= MAX_CLIPBOARD_TEXT_BYTES);
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
        Ok(read_flavor(self.handle.get_image())?.map(Content::Image))
    }

    pub fn write(&mut self, content: Content) -> Result<(), Error> {
        match content {
            Content::Text(text) => self.handle.set_text(text),
            Content::Html { html, text } => self.handle.set_html(html, Some(text)),
            Content::Image(image) => self.handle.set_image(image),
        }
    }

    pub fn write_image(&mut self, image: ImageData<'_>) -> Result<(), Error> {
        self.handle.set_image(image)
    }
}

fn read_flavor<T>(result: Result<T, Error>) -> Result<Option<T>, Error> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Error::ContentNotAvailable | Error::ConversionFailure) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use x11rb::connection::Connection;
    use x11rb::protocol::{
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::{ConnectionExt as _, CreateWindowAux, WindowClass},
        Event,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::COPY_DEPTH_FROM_PARENT;

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
