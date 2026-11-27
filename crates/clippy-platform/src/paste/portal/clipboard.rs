use super::Command;
use crate::clipboard::{Content, ImageData};
use anyhow::{anyhow, bail, ensure, Context, Result};
use ashpd::desktop::clipboard::{Clipboard, SelectionOwnerChanged, SetSelectionOptions};
use ashpd::desktop::remote_desktop::RemoteDesktop;
use ashpd::desktop::Session;
use clippy_core::clipboard::{
    strip_html_tags, validate_image_layout, MAX_CLIPBOARD_PNG_BYTES, MAX_CLIPBOARD_TEXT_BYTES,
};
use futures_util::StreamExt;
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, ImageEncoder, ImageFormat, ImageReader,
};
use std::{
    borrow::Cow,
    io::{Cursor, ErrorKind},
    os::fd::OwnedFd,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    time::Duration,
};
use tokio::{net::unix::pipe, sync::mpsc::Sender, time::timeout};
use zbus::{
    proxy::SignalStream,
    zvariant::{serialized::Context as DbusContext, to_bytes, OwnedObjectPath, LE},
};

const PNG: &str = "image/png";
const HTML: &str = "text/html";
const TEXT: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
const READ_TIME: Duration = Duration::from_secs(2);
const SIGNAL_TIME: Duration = Duration::from_secs(1);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(crate) struct Client {
    commands: Sender<Command>,
    generation: Arc<AtomicU64>,
}

impl Client {
    pub(super) fn new(commands: Sender<Command>, generation: Arc<AtomicU64>) -> Self {
        Self {
            commands,
            generation,
        }
    }

    pub(crate) fn generation(&self) -> Option<u64> {
        let value = self.generation.load(Ordering::Acquire);
        (value != 0).then_some(value)
    }

    pub(crate) fn read(&self) -> Result<Option<Content>> {
        self.call(Request::Read)?
            .map(|payload| payload.decode())
            .transpose()
    }

    pub(crate) fn write(&self, content: Content) -> Result<()> {
        match content {
            Content::Text(text) => self.publish(Payload::Text(text)),
            Content::Html { html, text } => self.publish(Payload::Html { html, text }),
            Content::Image(image) => self.write_image(image),
        }
    }

    pub(crate) fn write_image(&self, image: ImageData<'_>) -> Result<()> {
        let (width, height) = validate_image_layout(image.width, image.height, image.bytes.len())
            .map_err(anyhow::Error::msg)?;
        let mut png = Vec::with_capacity(image.bytes.len().min(1024 * 1024));
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
            .write_image(&image.bytes, width, height, ExtendedColorType::Rgba8)?;
        self.publish(Payload::Png(png))
    }

    fn publish(&self, payload: Payload) -> Result<()> {
        for mime in payload.types() {
            let bytes = payload.bytes(mime).expect("Advertised clipboard format");
            ensure!(
                bytes.len() <= limit(mime),
                "Clipboard data exceeds its size limit"
            );
        }
        self.call(|reply| Request::Publish(payload, reply))
    }

    fn call<T>(&self, request: impl FnOnce(Reply<T>) -> Request) -> Result<T> {
        ensure!(
            self.generation().is_some(),
            "Wayland clipboard permission is closed"
        );
        let (send, receive) = mpsc::sync_channel(1);
        self.commands
            .try_send(Command::Clipboard(request(send)))
            .map_err(|_| anyhow!("The Wayland clipboard is busy or closed"))?;
        receive
            .recv()
            .context("The Wayland clipboard service closed")?
    }
}

type Reply<T> = SyncSender<Result<T>>;

pub(super) enum Request {
    Read(Reply<Option<Arc<Payload>>>),
    Publish(Payload, Reply<()>),
}

impl Request {
    pub(super) fn closed(self) {
        let error = anyhow!("Wayland clipboard permission is closed");
        match self {
            Self::Read(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Publish(_, reply) => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

pub(super) enum Payload {
    Text(String),
    Html { html: String, text: String },
    Png(Vec<u8>),
}

impl Payload {
    fn types(&self) -> &[&str] {
        match self {
            Self::Text(_) => &TEXT,
            Self::Html { .. } => &[HTML, TEXT[0], TEXT[1], TEXT[2]],
            Self::Png(_) => &[PNG],
        }
    }

    fn bytes(&self, mime: &str) -> Option<&[u8]> {
        match self {
            Self::Text(text) if TEXT.contains(&mime) => Some(text.as_bytes()),
            Self::Html { html, .. } if mime == HTML => Some(html.as_bytes()),
            Self::Html { text, .. } if TEXT.contains(&mime) => Some(text.as_bytes()),
            Self::Png(png) if mime == PNG => Some(png),
            _ => None,
        }
    }

    fn decode(&self) -> Result<Content> {
        Ok(match self {
            Self::Text(text) => Content::Text(text.clone()),
            Self::Html { html, text } => Content::Html {
                html: html.clone(),
                text: text.clone(),
            },
            Self::Png(png) => {
                let png_reader = || ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
                let (width, height) = png_reader().into_dimensions()?;
                let (width, height) = (width as usize, height as usize);
                let len = width
                    .checked_mul(height)
                    .and_then(|n| n.checked_mul(4))
                    .context("PNG dimensions overflow")?;
                validate_image_layout(width, height, len).map_err(anyhow::Error::msg)?;
                let bytes = png_reader().decode()?.into_rgba8().into_raw();
                Content::Image(ImageData {
                    width,
                    height,
                    bytes: Cow::Owned(bytes),
                })
            }
        })
    }
}

enum Selection {
    External(Vec<String>),
    Local(Arc<Payload>),
}

pub(super) enum Event {
    Closed,
    Owner(SelectionOwnerChanged),
    Transfer(String, u32),
}

pub(super) struct Binding {
    proxy: Clipboard,
    events: SignalStream<'static>,
    closed: SignalStream<'static>,
    path: OwnedObjectPath,
    generation: Arc<AtomicU64>,
    selection: Selection,
}

impl Binding {
    pub(super) async fn prepare(
        connection: zbus::Connection,
        session: &Session<RemoteDesktop>,
        generation: Arc<AtomicU64>,
    ) -> Result<Self> {
        let proxy = Clipboard::with_connection(connection.clone()).await?;
        let path: OwnedObjectPath = to_bytes(DbusContext::new_dbus(LE, 0), session)?
            .deserialize()?
            .0;
        // 在 Start 前订阅，保留系统报告的初始所有者。
        let events = proxy.receive_all_signals().await?;
        let closed = zbus::Proxy::new_owned(
            connection,
            "org.freedesktop.portal.Desktop",
            path.clone(),
            "org.freedesktop.portal.Session",
        )
        .await?
        .receive_signal("Closed")
        .await?;
        proxy.request(session, Default::default()).await?;
        Ok(Self {
            proxy,
            events,
            closed,
            path,
            generation,
            selection: Selection::External(Vec::new()),
        })
    }

    pub(super) fn advance(&self) {
        self.generation.store(
            NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            Ordering::Release,
        );
    }

    pub(super) async fn next(&mut self) -> Event {
        next_event(&mut self.events, &mut self.closed, &self.path).await
    }

    pub(super) async fn observe(&mut self, session: &Session<RemoteDesktop>, event: Event) {
        match event {
            Event::Closed => self.generation.store(0, Ordering::Release),
            Event::Owner(owner) if owner.session_is_owner() != Some(true) => {
                self.selection = Selection::External(owner.mime_types().to_vec());
                self.advance();
            }
            Event::Owner(_) => {}
            Event::Transfer(mime, serial) => self.transfer(session, &mime, serial).await,
        }
    }

    pub(super) async fn command(&mut self, session: &Session<RemoteDesktop>, request: Request) {
        match request {
            Request::Read(reply) => {
                let _ = reply.send(self.read(session).await);
            }
            Request::Publish(payload, reply) => {
                let result = self.publish(session, payload).await;
                if result.is_err() {
                    // 写入能否生效已不可确认，迟到的所有者事件不能用于下一次发布。
                    self.generation.store(0, Ordering::Release);
                }
                let _ = reply.send(result);
            }
        }
    }

    async fn read(&mut self, session: &Session<RemoteDesktop>) -> Result<Option<Arc<Payload>>> {
        let mimes = match &self.selection {
            Selection::Local(payload) => return Ok(Some(payload.clone())),
            Selection::External(mimes) => mimes,
        };
        let outcome = tokio::select! {
            biased;
            notice = next_event(&mut self.events, &mut self.closed, &self.path) => Err(notice),
            result = timeout(READ_TIME, read_external(&self.proxy, session, mimes)) => Ok(result),
        };
        match outcome {
            Ok(result) => Ok(result
                .context("Reading the Wayland clipboard timed out")??
                .map(Arc::new)),
            Err(notice) => {
                self.observe(session, notice).await;
                // 调用方以原代次丢弃读结果，等下一次所有者一致的读取。
                bail!("Wayland clipboard changed while reading")
            }
        }
    }

    async fn publish(&mut self, session: &Session<RemoteDesktop>, payload: Payload) -> Result<()> {
        let options = SetSelectionOptions::default().set_mime_types(payload.types());
        timeout(SIGNAL_TIME, self.proxy.set_selection(session, options))
            .await
            .context("Publishing the Wayland clipboard timed out")??;
        // 等同一个会话的所有者确认，旧所有者的通知不能作为本次复制成功。
        timeout(SIGNAL_TIME, async {
            loop {
                match self.next().await {
                    Event::Closed => {
                        self.generation.store(0, Ordering::Release);
                        bail!("Wayland clipboard permission is closed");
                    }
                    Event::Owner(owner) if owner.session_is_owner() == Some(true) => {
                        self.selection = Selection::Local(Arc::new(payload));
                        self.advance();
                        return Ok(());
                    }
                    notice => self.observe(session, notice).await,
                }
            }
        })
        .await
        .context("The desktop did not confirm the clipboard selection")?
    }

    async fn transfer(&self, session: &Session<RemoteDesktop>, mime: &str, serial: u32) {
        let bytes = match &self.selection {
            Selection::Local(payload) => payload.bytes(mime),
            Selection::External(_) => None,
        };
        let sent = if let Some(bytes) = bytes {
            matches!(
                timeout(READ_TIME, async {
                    let descriptor = self.proxy.selection_write(session, serial).await?;
                    write_pipe(descriptor.into(), bytes).await
                })
                .await,
                Ok(Ok(()))
            )
        } else {
            false
        };
        let _ = timeout(
            SIGNAL_TIME,
            self.proxy.selection_write_done(session, serial, sent),
        )
        .await;
    }
}

async fn next_event(
    events: &mut SignalStream<'static>,
    closed: &mut SignalStream<'static>,
    path: &OwnedObjectPath,
) -> Event {
    loop {
        let message = tokio::select! {
            biased;
            _ = closed.next() => return Event::Closed,
            signal = events.next() => match signal {
                Some(message) => message,
                None => return Event::Closed,
            },
        };
        let body = message.body();
        match message.header().member().map(|name| name.as_str()) {
            Some("SelectionOwnerChanged") => {
                if let Ok((owner, options)) =
                    body.deserialize::<(OwnedObjectPath, SelectionOwnerChanged)>()
                {
                    if &owner == path {
                        return Event::Owner(options);
                    }
                }
            }
            Some("SelectionTransfer") => {
                if let Ok((owner, mime, serial)) =
                    body.deserialize::<(OwnedObjectPath, String, u32)>()
                {
                    if &owner == path {
                        return Event::Transfer(mime, serial);
                    }
                }
            }
            _ => {}
        }
    }
}

fn limit(mime: &str) -> usize {
    if mime == PNG {
        MAX_CLIPBOARD_PNG_BYTES
    } else {
        MAX_CLIPBOARD_TEXT_BYTES
    }
}

async fn read_external(
    clipboard: &Clipboard,
    session: &Session<RemoteDesktop>,
    mimes: &[String],
) -> Result<Option<Payload>> {
    let has = |mime: &str| mimes.iter().any(|offered| offered == mime);
    let text = match TEXT.into_iter().find(|mime| has(mime)) {
        Some(mime) => Some(String::from_utf8(
            read_format(clipboard, session, mime).await?,
        )?),
        None => None,
    }
    .filter(|text| !text.is_empty());
    if has(HTML) {
        let html = String::from_utf8(read_format(clipboard, session, HTML).await?)?;
        if !html.is_empty() {
            let text = text.unwrap_or_else(|| strip_html_tags(&html));
            return Ok(Some(Payload::Html { html, text }));
        }
    }
    if let Some(text) = text {
        return Ok(Some(Payload::Text(text)));
    }
    if has(PNG) {
        let png = read_format(clipboard, session, PNG).await?;
        return Ok((!png.is_empty()).then_some(Payload::Png(png)));
    }
    Ok(None)
}

async fn read_format(
    clipboard: &Clipboard,
    session: &Session<RemoteDesktop>,
    mime: &str,
) -> Result<Vec<u8>> {
    let pipe =
        pipe::Receiver::from_owned_fd(clipboard.selection_read(session, mime).await?.into())?;
    let mut data = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        pipe.readable().await?;
        let count = match pipe.try_read(&mut chunk) {
            Err(error) if error.kind() == ErrorKind::WouldBlock => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(data);
        }
        ensure!(
            data.len() + count <= limit(mime),
            "Clipboard data exceeds its size limit"
        );
        data.extend_from_slice(&chunk[..count]);
    }
}

async fn write_pipe(descriptor: OwnedFd, mut bytes: &[u8]) -> Result<()> {
    let pipe = pipe::Sender::from_owned_fd(descriptor)?;
    while !bytes.is_empty() {
        pipe.writable().await?;
        let count = match pipe.try_write(bytes) {
            Err(error) if error.kind() == ErrorKind::WouldBlock => continue,
            result => result?,
        };
        ensure!(count != 0, "Clipboard pipe closed while writing");
        bytes = &bytes[count..];
    }
    Ok(())
}
