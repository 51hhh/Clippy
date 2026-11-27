use super::Offer;
use anyhow::{anyhow, bail, ensure, Result};
use raw_window_handle::{DisplayHandle, RawDisplayHandle};
use rustix::{
    event::{poll, PollFd, PollFlags, Timespec},
    fs::{fcntl_getfl, fcntl_setfl, OFlags},
    io::Errno,
};
use std::{
    collections::HashMap,
    ffi::c_void,
    fs::File,
    io::{ErrorKind, Write},
    os::fd::OwnedFd,
    ptr::NonNull,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use wayland_client::{
    backend::{Backend, ObjectId, WaylandError},
    event_created_child,
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::WlDataDeviceManager,
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_keyboard::{self, WlKeyboard},
        wl_pointer::{self, WlPointer},
        wl_registry::{self, WlRegistry},
        wl_seat::{self, WlSeat},
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};

const MARKER: &str = "application/x-clippy-copy-";
const CONFIRM_LIMIT: Duration = Duration::from_secs(1);
const TRANSFER_LIMIT: Duration = Duration::from_secs(2);
const TRANSFER_QUEUE: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    seat: u32,
    event: u64,
    serial: u32,
}

pub struct Publisher {
    queue: EventQueue<State>,
    state: State,
}

impl Publisher {
    /// # Safety
    /// 销毁 Publisher 前必须保留窗口系统的 Wayland 显示连接。
    pub unsafe fn attach(display: DisplayHandle<'_>) -> Result<Self> {
        let RawDisplayHandle::Wayland(display) = display.as_raw() else {
            bail!("The window does not use Wayland");
        };
        // 注册独立队列并借用同一个客户端，输入序号只对这条连接有效。
        let connection = Connection::from_backend(unsafe {
            Backend::from_foreign_display(display.display.as_ptr().cast())
        });
        let queue = connection.new_event_queue();
        let registry = connection.display().get_registry(&queue.handle(), ());
        let mut this = Self {
            queue,
            state: State::new(registry),
        };
        this.dispatch()?;
        Ok(this)
    }

    pub fn dispatch(&mut self) -> Result<()> {
        self.queue.dispatch_pending(&mut self.state)?;
        self.state.expire();
        match self.queue.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn ticket(&self, surface: NonNull<c_void>) -> Option<Ticket> {
        let (name, seat) = self.state.active_seat()?;
        let focus = seat.focus.as_ref()?.as_ptr().cast::<c_void>();
        (focus == surface.as_ptr()).then_some(Ticket {
            seat: name,
            event: seat.event,
            serial: seat.serial,
        })
    }

    pub fn publish(&mut self, ticket: Ticket, offer: Offer) -> Result<()> {
        self.dispatch()?;
        offer.validate()?;
        ensure!(
            self.state.pending.is_none(),
            "Clipboard is busy. Try again."
        );
        let (name, seat) = self
            .state
            .active_seat()
            .ok_or_else(|| anyhow!("Focus the Wayland window before copying"))?;
        ensure!(
            name == ticket.seat && seat.event == ticket.event && seat.serial == ticket.serial,
            "The clipboard window lost focus or received newer input"
        );
        let manager = self
            .state
            .manager
            .as_ref()
            .ok_or_else(|| anyhow!("The desktop does not provide a Wayland data device"))?;
        let device = seat
            .device
            .clone()
            .ok_or_else(|| anyhow!("The clipboard seat changed"))?;
        self.state.publication += 1;
        let id = self.state.publication;
        let offer = Arc::new(offer);
        let source = manager.create_data_source(
            &self.queue.handle(),
            Source {
                id,
                offer: offer.clone(),
            },
        );
        for mime in offer.types() {
            source.offer((*mime).to_owned());
        }
        source.offer(format!("{MARKER}{}-{id}", std::process::id()));
        device.set_selection(Some(&source), ticket.serial);
        self.state.pending = Some(Pending {
            id,
            seat: name,
            source,
            expires: Instant::now() + CONFIRM_LIMIT,
        });
        self.dispatch()
    }

    pub fn take_result(&mut self) -> Option<Result<()>> {
        self.state.result.take()
    }

    pub fn take_transfers(&mut self) -> Vec<Transfer> {
        std::mem::take(&mut self.state.transfers)
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        if let Some(pending) = self.state.pending.take() {
            pending.source.destroy();
        }
        if let Some(active) = self.state.active.take() {
            active.destroy();
        }
        self.state.seats.clear();
        let _ = self.queue.flush();
    }
}

pub struct Transfer {
    fd: OwnedFd,
    mime: String,
    offer: Arc<Offer>,
}

impl Transfer {
    pub fn deliver(self) -> Result<()> {
        let mut bytes = self
            .offer
            .bytes(&self.mime)
            .expect("Selected clipboard format");
        fcntl_setfl(&self.fd, fcntl_getfl(&self.fd)? | OFlags::NONBLOCK)?;
        let mut pipe = File::from(self.fd);
        let deadline = Instant::now() + TRANSFER_LIMIT;
        while !bytes.is_empty() {
            match pipe.write(bytes) {
                Ok(0) => bail!("Clipboard pipe closed while writing"),
                Ok(written) => bytes = &bytes[written..],
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    ensure!(!remaining.is_zero(), "Clipboard transfer timed out");
                    let time = Timespec {
                        tv_sec: remaining.as_secs() as i64,
                        tv_nsec: i64::from(remaining.subsec_nanos()),
                    };
                    match poll(&mut [PollFd::new(&pipe, PollFlags::OUT)], Some(&time)) {
                        Ok(0) => bail!("Clipboard transfer timed out"),
                        Ok(_) | Err(Errno::INTR) => (),
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

struct Source {
    id: u64,
    offer: Arc<Offer>,
}

struct Pending {
    id: u64,
    seat: u32,
    source: WlDataSource,
    expires: Instant,
}

#[derive(Default)]
struct Seat {
    seat: Option<WlSeat>,
    device: Option<WlDataDevice>,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    focus: Option<ObjectId>,
    pointing: Option<ObjectId>,
    event: u64,
    serial: u32,
}

impl Seat {
    fn clear_focus(&mut self) {
        self.focus = None;
        self.serial = 0;
    }

    fn release_keyboard(&mut self) {
        if let Some(keyboard) = self.keyboard.take().filter(|proxy| proxy.version() >= 3) {
            keyboard.release();
        }
        self.clear_focus();
    }

    fn release_pointer(&mut self) {
        if let Some(pointer) = self.pointer.take().filter(|proxy| proxy.version() >= 3) {
            pointer.release();
        }
        self.pointing = None;
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.release_keyboard();
        self.release_pointer();
        if let Some(device) = self.device.take().filter(|proxy| proxy.version() >= 2) {
            device.release();
        }
        if let Some(seat) = self.seat.take().filter(|proxy| proxy.version() >= 5) {
            seat.release();
        }
    }
}

struct State {
    _registry: WlRegistry,
    manager: Option<WlDataDeviceManager>,
    seats: HashMap<u32, Seat>,
    event: u64,
    publication: u64,
    pending: Option<Pending>,
    active: Option<WlDataSource>,
    result: Option<Result<()>>,
    transfers: Vec<Transfer>,
}

impl State {
    fn new(registry: WlRegistry) -> Self {
        Self {
            _registry: registry,
            manager: None,
            seats: HashMap::new(),
            event: 0,
            publication: 0,
            pending: None,
            active: None,
            result: None,
            transfers: Vec::new(),
        }
    }

    fn active_seat(&self) -> Option<(u32, &Seat)> {
        self.seats
            .iter()
            .filter(|(_, seat)| seat.serial != 0 && seat.focus.is_some())
            .max_by_key(|(_, seat)| seat.event)
            .map(|(&name, seat)| (name, seat))
    }

    fn note_input(&mut self, name: u32, serial: u32) {
        let seat = self.seats.get_mut(&name).expect("Bound Wayland seat");
        self.event += 1;
        seat.event = self.event;
        seat.serial = serial;
    }

    fn expire(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| Instant::now() >= pending.expires)
        {
            let pending = self.pending.take().unwrap();
            pending.source.destroy();
            self.result = Some(Err(anyhow!(
                "The desktop did not confirm the clipboard selection"
            )));
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" => {
                let seat = registry.bind::<WlSeat, _, Self>(name, version.min(7), queue, name);
                let device = state
                    .manager
                    .as_ref()
                    .map(|mgr| mgr.get_data_device(&seat, queue, name));
                let bound = state.seats.entry(name).or_default();
                bound.seat = Some(seat);
                bound.device = device;
            }
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_data_device_manager" => {
                let manager =
                    registry.bind::<WlDataDeviceManager, _, Self>(name, version.min(3), queue, ());
                for (&name, seat) in &mut state.seats {
                    seat.device =
                        Some(manager.get_data_device(seat.seat.as_ref().unwrap(), queue, name));
                }
                state.manager = Some(manager);
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.seats.remove(&name);
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(State: ignore WlDataDeviceManager);

impl Dispatch<WlSeat, u32> for State {
    fn event(
        state: &mut Self,
        proxy: &WlSeat,
        event: wl_seat::Event,
        name: &u32,
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        let Some(seat) = state.seats.get_mut(name) else {
            return;
        };
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Keyboard) {
                seat.keyboard
                    .get_or_insert_with(|| proxy.get_keyboard(queue, *name));
            } else {
                seat.release_keyboard();
            }
            if caps.contains(wl_seat::Capability::Pointer) {
                seat.pointer
                    .get_or_insert_with(|| proxy.get_pointer(queue, *name));
            } else {
                seat.release_pointer();
            }
        }
    }
}

impl Dispatch<WlKeyboard, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = state.seats.get_mut(name) else {
            return;
        };
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                seat.clear_focus();
                seat.focus = Some(surface.id());
            }
            wl_keyboard::Event::Leave { .. } => seat.clear_focus(),
            wl_keyboard::Event::Key {
                serial,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } if seat.focus.is_some() => {
                state.note_input(*name, serial);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = state.seats.get_mut(name) else {
            return;
        };
        match event {
            wl_pointer::Event::Enter { surface, .. } => seat.pointing = Some(surface.id()),
            wl_pointer::Event::Leave { .. } => seat.pointing = None,
            wl_pointer::Event::Button {
                serial,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if seat.focus.is_some() && seat.focus == seat.pointing => {
                state.note_input(*name, serial);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataDevice, u32> for State {
    event_created_child!(State, WlDataDevice, [
        0 => (WlDataOffer, AtomicU64::new(0))
    ]);

    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        seat: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Selection { id: Some(offer) } => {
                let id = offer
                    .data::<AtomicU64>()
                    .expect("Bound data offer")
                    .load(Ordering::Relaxed);
                offer.destroy();
                if state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.id == id && pending.seat == *seat)
                {
                    let pending = state.pending.take().unwrap();
                    if let Some(old) = state.active.replace(pending.source) {
                        old.destroy();
                    }
                    state.result = Some(Ok(()));
                }
            }
            wl_data_device::Event::Enter {
                id: Some(offer), ..
            } => offer.destroy(),
            _ => {}
        }
    }
}

impl Dispatch<WlDataOffer, AtomicU64> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        id: &AtomicU64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            if let Some((process, serial)) = mime_type
                .strip_prefix(MARKER)
                .and_then(|mime| mime.split_once('-'))
            {
                if process.parse::<u32>() == Ok(std::process::id()) {
                    if let Ok(serial) = serial.parse() {
                        id.store(serial, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

impl Dispatch<WlDataSource, Source> for State {
    fn event(
        state: &mut Self,
        proxy: &WlDataSource,
        event: wl_data_source::Event,
        source: &Source,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                if state.transfers.len() < TRANSFER_QUEUE
                    && source.offer.bytes(&mime_type).is_some()
                {
                    state.transfers.push(Transfer {
                        fd,
                        mime: mime_type,
                        offer: source.offer.clone(),
                    });
                }
            }
            wl_data_source::Event::Cancelled => {
                proxy.destroy();
                if state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.id == source.id)
                {
                    state.pending.take();
                    state.result = Some(Err(anyhow!("The clipboard selection was replaced")));
                }
                if state.active.as_ref() == Some(proxy) {
                    state.active.take();
                }
            }
            _ => {}
        }
    }
}
