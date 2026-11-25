use super::error::PasteError;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageData, ClientMessageEvent, ConnectionExt, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

fn protocol(error: impl std::fmt::Display) -> PasteError {
    PasteError::X11Protocol(error.to_string())
}

pub(super) fn active_window() -> Result<Window, PasteError> {
    let (connection, screen) = RustConnection::connect(None).map_err(protocol)?;
    active_window_on(&connection, screen)
}

pub(super) fn paste(target: Window) -> Result<(), PasteError> {
    activate_and_confirm(target)?;
    simulate_ctrl_v()
}

fn activate_and_confirm(target: Window) -> Result<(), PasteError> {
    let (connection, screen) = RustConnection::connect(None).map_err(protocol)?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or(PasteError::X11ScreenMissing)?
        .root;
    let active_atom = atom(&connection, b"_NET_ACTIVE_WINDOW")?;
    let message = ClientMessageEvent::new(
        32,
        target,
        active_atom,
        ClientMessageData::from([1, CURRENT_TIME, 0, 0, 0]),
    );
    connection
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            message,
        )
        .map_err(protocol)?;
    connection.flush().map_err(protocol)?;

    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if active_window_on(&connection, screen).ok() == Some(target) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(PasteError::X11FocusNotRestored)
}

fn simulate_ctrl_v() -> Result<(), PasteError> {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard, Settings,
    };

    let injection = |action: &str| {
        let action = action.to_string();
        move |error: enigo::InputError| PasteError::KeyInjection {
            action,
            detail: error.to_string(),
        }
    };

    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|error| PasteError::InputBackendUnavailable(error.to_string()))?;
    enigo
        .key(Key::Control, Press)
        .map_err(injection("按下 Control"))?;
    let click = enigo.key(Key::Unicode('v'), Click);
    let release = enigo.key(Key::Control, Release);
    click.map_err(injection("按下 V"))?;
    release.map_err(injection("释放 Control"))?;
    Ok(())
}

fn active_window_on(connection: &RustConnection, screen: usize) -> Result<Window, PasteError> {
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or(PasteError::X11ScreenMissing)?
        .root;
    let active_atom = atom(connection, b"_NET_ACTIVE_WINDOW")?;
    let reply = connection
        .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)
        .map_err(protocol)?
        .reply()
        .map_err(protocol)?;
    reply
        .value32()
        .and_then(|mut values| values.next())
        .filter(|window| *window != 0)
        .ok_or(PasteError::X11ActiveWindowEmpty)
}

fn atom(connection: &RustConnection, name: &[u8]) -> Result<u32, PasteError> {
    let atom = connection
        .intern_atom(false, name)
        .map_err(protocol)?
        .reply()
        .map_err(protocol)?
        .atom;
    if atom == 0 {
        Err(PasteError::X11AtomMissing(
            String::from_utf8_lossy(name).to_string(),
        ))
    } else {
        Ok(atom)
    }
}
