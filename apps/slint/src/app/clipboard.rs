use anyhow::{anyhow, Context, Result};
use clippy_capture::png_io::encode_png;
use clippy_core::clipboard::{
    compute_hash, is_sensitive_text, rgba_fingerprint, strip_html_tags, validate_image_layout,
    MAX_CLIPBOARD_PNG_BYTES,
};
use clippy_core::models::{ClipItem, ContentType};
use clippy_platform::{
    clipboard::{Clipboard, ClipboardWake, Content, ImageData},
    paste::{Paste, Target},
};
use clippy_storage::{BoundedImageData, StorageClient, StorageEngine, StorageError};
use image::{ImageFormat, ImageReader};
use std::borrow::Cow;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::{post, Event};

const POLL: Duration = Duration::from_millis(250);
const EXPIRE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_PNG_BYTES: usize = MAX_CLIPBOARD_PNG_BYTES;

pub(super) enum Request {
    Copy {
        id: i64,
        target: Option<Target>,
    },
    Paste {
        target: Target,
        generation: Option<u64>,
    },
    WriteImage {
        image: Arc<image::RgbaImage>,
        finish: Box<dyn FnOnce(Result<()>) + Send>,
    },
    WriteText {
        text: String,
        finish: Box<dyn FnOnce(Result<()>) + Send>,
    },
    Authorize,
    ClosePermission,
    #[cfg(target_os = "linux")]
    ForgetPermission,
}

pub(super) struct Worker {
    sender: Option<SyncSender<Request>>,
    wake: Option<ClipboardWake>,
    thread: Option<JoinHandle<()>>,
    limit: Arc<AtomicU32>,
}

impl Worker {
    pub(super) fn start(
        storage: StorageClient,
        data: PathBuf,
        limit: u32,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(3);
        let limit = Arc::new(AtomicU32::new(limit));
        let worker_limit = limit.clone();
        let wake = ClipboardWake::new();
        let worker_wake = wake.clone();
        let thread = thread::Builder::new()
            .name("clippy-clipboard".into())
            .spawn(move || watch(receiver, worker_wake, storage, data, &worker_limit))?;
        Ok(Self {
            sender: Some(sender),
            wake,
            thread: Some(thread),
            limit,
        })
    }

    pub(super) fn submit(&self, request: Request) -> Result<()> {
        self.sender
            .as_ref()
            .expect("Clipboard worker running")
            .try_send(request)
            .map_err(|_| anyhow!("Clipboard is busy. Try again."))?;
        self.notify();
        Ok(())
    }

    fn notify(&self) {
        if let Some(wake) = &self.wake {
            wake.notify();
        }
    }

    pub(super) fn set_limit(&self, limit: u32) {
        self.limit.store(limit, Ordering::Relaxed);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.sender.take();
        self.notify();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn wait_for_request(
    receiver: &Receiver<Request>,
    clipboard: Option<&mut Clipboard>,
    wake: Option<&ClipboardWake>,
    timeout: Duration,
) -> Result<Request, RecvTimeoutError> {
    let mut wait = POLL;
    if let (Some(clipboard), Some(wake)) = (clipboard, wake) {
        // 一次系统唤醒可能对应多个请求，队列还有待办时继续处理。
        match receiver.try_recv() {
            Ok(request) => return Ok(request),
            Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
            Err(TryRecvError::Empty) => (),
        }
        if clipboard.wait_for_activity(wake, timeout) {
            wait = Duration::ZERO;
        }
    }
    receiver.recv_timeout(wait)
}

fn watch(
    receiver: Receiver<Request>,
    wake: Option<ClipboardWake>,
    storage: StorageClient,
    data: PathBuf,
    limit: &AtomicU32,
) {
    let mut paste = Paste::new(&data);
    let mut clipboard = None;
    let mut generation = None;
    let mut previous = None;
    let mut retry = false;
    let mut expire = Instant::now();
    let mut reported = String::new();
    loop {
        let until_expiry = expire.saturating_duration_since(Instant::now());
        let timeout = if retry {
            POLL.min(until_expiry)
        } else {
            until_expiry
        };
        let request = match wait_for_request(&receiver, clipboard.as_mut(), wake.as_ref(), timeout)
        {
            Ok(request) => Some(request),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        #[cfg(target_os = "linux")]
        {
            if clipboard.as_ref().is_some_and(Clipboard::portal_closed) {
                clipboard.take();
                generation = None;
                previous = None;
                retry = false;
            }
            if clipboard.is_none() {
                clipboard = paste.clipboard();
            }
        }

        match request {
            Some(Request::Authorize) => {
                #[cfg(target_os = "linux")]
                if clippy_platform::session::is_wayland() {
                    if clipboard.is_none() {
                        clipboard = Clipboard::open().ok();
                    }
                    if clipboard.is_none() || paste.clipboard().is_some() {
                        paste.authorize_clipboard(Box::new(|result| {
                            post(Event::Permission(result, true));
                        }));
                        continue;
                    }
                }
                paste.authorize(Box::new(|result| post(Event::Permission(result, false))));
                continue;
            }
            Some(Request::ClosePermission) => {
                #[cfg(target_os = "linux")]
                if paste.clipboard().is_some() {
                    // 自动粘贴开关只停用输入，已获准的剪贴板仍由同一会话持有。
                    continue;
                }
                paste.close();
                continue;
            }
            #[cfg(target_os = "linux")]
            Some(Request::ForgetPermission) => {
                post(Event::PermissionForgotten(paste.forget_permission()));
                continue;
            }
            request => {
                let system = match clipboard.as_mut() {
                    Some(system) => system,
                    None => match Clipboard::open() {
                        Ok(system) => clipboard.insert(system),
                        Err(error) => {
                            report_error(&mut reported, format!("Clipboard unavailable: {error}"));
                            match request {
                                Some(Request::Copy { .. }) => {
                                    post(Event::Copied(Err(anyhow!(error))))
                                }
                                Some(Request::Paste { .. }) => {
                                    post(Event::Pasted(Err(anyhow!(error))))
                                }
                                Some(
                                    Request::WriteImage { finish, .. }
                                    | Request::WriteText { finish, .. },
                                ) => finish(Err(anyhow!(error))),
                                _ => {}
                            }
                            continue;
                        }
                    },
                };
                match request {
                    Some(Request::Copy { id, target }) => {
                        let result = load_clip(&storage, id)
                            .and_then(|content| system.write(content).map_err(Into::into));
                        retry |= result.is_ok();
                        post(Event::Copied(
                            result.map(|()| (target, system.generation())),
                        ));
                    }
                    Some(Request::Paste { target, generation }) => {
                        if generation.is_some() && system.generation() != generation {
                            post(Event::Pasted(Err(anyhow!(
                                "Clipboard changed before paste"
                            ))));
                        } else {
                            paste.inject(target, Box::new(|result| post(Event::Pasted(result))));
                        }
                    }
                    Some(Request::WriteImage { image, finish }) => {
                        let (width, height) = image.dimensions();
                        let result = system
                            .write_image(ImageData {
                                width: width as usize,
                                height: height as usize,
                                bytes: Cow::Borrowed(image.as_raw()),
                            })
                            .map_err(Into::into);
                        retry |= result.is_ok();
                        finish(result);
                    }
                    Some(Request::WriteText { text, finish }) => {
                        let result = system.write(Content::Text(text)).map_err(Into::into);
                        retry |= result.is_ok();
                        finish(result);
                    }
                    Some(Request::Authorize | Request::ClosePermission) => unreachable!(),
                    #[cfg(target_os = "linux")]
                    Some(Request::ForgetPermission) => unreachable!(),
                    None => {}
                }

                let current = system.generation();
                if current.is_none() || generation != current || retry {
                    if current.is_some() && current != generation {
                        previous = None;
                    }
                    generation = current;
                    retry = match system.read() {
                        Ok(Some(content)) => {
                            let now = system.generation();
                            if current.is_some() && current != now {
                                true
                            } else {
                                match store_snapshot(
                                    &storage,
                                    content,
                                    &mut previous,
                                    limit.load(Ordering::Relaxed),
                                ) {
                                    Ok(()) => false,
                                    Err(error) => {
                                        report_error(
                                            &mut reported,
                                            format!(
                                                "Could not record clipboard history: {error:#}"
                                            ),
                                        );
                                        true
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            previous = None;
                            false
                        }
                        Err(error) => {
                            report_error(
                                &mut reported,
                                format!("Could not read clipboard: {error}"),
                            );
                            true
                        }
                    };
                }
            }
        }
        if Instant::now() >= expire {
            if let Ok(removed) = query(&storage, |database| database.purge_expired_sensitive(300)) {
                if !removed.is_empty() {
                    post(Event::HistoryChanged);
                }
            }
            expire = Instant::now() + EXPIRE_INTERVAL;
        }
    }
}

fn report_error(previous: &mut String, error: String) {
    if *previous != error {
        eprintln!("{error}");
        previous.clone_from(&error);
        post(Event::ClipboardStatus(error));
    }
}

fn query<T: Send + 'static>(
    storage: &StorageClient,
    work: impl FnOnce(&StorageEngine) -> Result<T, StorageError> + Send + 'static,
) -> Result<T> {
    let (send, receive) = mpsc::sync_channel(1);
    storage.execute(work, move |result| {
        let _ = send.send(result);
    })?;
    Ok(receive.recv()??)
}

fn load_clip(storage: &StorageClient, id: i64) -> Result<Content> {
    let (detail, image) = query(storage, move |database| {
        let detail = database.get_clip_detail_by_id(id)?;
        let image = if detail.content_type == ContentType::Image {
            database.get_bounded_image_snapshot(id, MAX_PNG_BYTES)?
        } else {
            None
        };
        Ok((detail, image))
    })?;
    match detail {
        ClipItem {
            content_type: ContentType::Text,
            text_content: Some(text),
            ..
        } => Ok(Content::Text(text)),
        ClipItem {
            content_type: ContentType::Html,
            html_content: Some(html),
            text_content,
            ..
        } => Ok(Content::Html {
            text: text_content.unwrap_or_else(|| strip_html_tags(&html)),
            html,
        }),
        ClipItem {
            content_type: ContentType::Image,
            ..
        } => {
            let Some(snapshot) = image else {
                unreachable!()
            };
            let BoundedImageData::Bytes(bytes) = snapshot.image else {
                return Err(anyhow!(
                    "Image content is unavailable or exceeds the clipboard budget"
                ));
            };
            let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(16_384);
            limits.max_image_height = Some(16_384);
            limits.max_alloc = Some(192 * 1024 * 1024);
            reader.limits(limits);
            let decoded = reader
                .decode()
                .context("Cannot decode clipboard image")?
                .into_rgba8();
            let (width, height) = decoded.dimensions();
            validate_image_layout(width as usize, height as usize, decoded.as_raw().len())
                .map_err(anyhow::Error::msg)?;
            Ok(Content::Image(arboard_image(
                width,
                height,
                decoded.into_raw(),
            )))
        }
        _ => Err(anyhow!("Clipboard content is missing")),
    }
}

fn arboard_image(width: u32, height: u32, pixels: Vec<u8>) -> ImageData<'static> {
    ImageData {
        width: width as usize,
        height: height as usize,
        bytes: Cow::Owned(pixels),
    }
}

#[derive(PartialEq, Eq)]
enum Fingerprint {
    Text(String),
    Html(String),
    Image(u64),
}

fn store_snapshot(
    storage: &StorageClient,
    content: Content,
    previous: &mut Option<Fingerprint>,
    limit: u32,
) -> Result<()> {
    let identity = match &content {
        Content::Text(text) => Fingerprint::Text(compute_hash(text.as_bytes())),
        Content::Html { html, .. } => Fingerprint::Html(compute_hash(html.as_bytes())),
        Content::Image(image) => {
            if validate_image_layout(image.width, image.height, image.bytes.len()).is_err() {
                *previous = None;
                return Ok(());
            }
            Fingerprint::Image(rgba_fingerprint(image.width, image.height, &image.bytes))
        }
    };
    if previous.as_ref() == Some(&identity) {
        return Ok(());
    }
    let (kind, text, html, png, hash, size) = match content {
        Content::Text(text) => {
            let Fingerprint::Text(hash) = &identity else {
                unreachable!()
            };
            let size = text.len();
            (
                ContentType::Text,
                Some(text),
                None,
                None,
                hash.clone(),
                size,
            )
        }
        Content::Html { html, text } => {
            let Fingerprint::Html(hash) = &identity else {
                unreachable!()
            };
            let size = html.len();
            (
                ContentType::Html,
                Some(text),
                Some(html),
                None,
                hash.clone(),
                size,
            )
        }
        Content::Image(image) => {
            let png = encode_png(&image.bytes, image.width as u32, image.height as u32)?;
            let size = png.len();
            let hash = compute_hash(&png);
            (ContentType::Image, None, None, Some(png), hash, size)
        }
    };
    let sensitive = text.as_deref().is_some_and(is_sensitive_text);
    query(storage, move |database| {
        database.insert_clip(
            &kind,
            text.as_deref(),
            html.as_deref(),
            png.as_deref(),
            &hash,
            size as i64,
            sensitive,
        )?;
        let _ = database.cleanup_old_entries(limit);
        Ok(())
    })?;
    *previous = Some(identity);
    post(Event::HistoryChanged);
    Ok(())
}
