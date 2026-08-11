use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageData, ClientMessageEvent, ConnectionExt, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

pub(super) fn active_window() -> Result<Window, String> {
    let (connection, screen) = RustConnection::connect(None).map_err(|error| error.to_string())?;
    active_window_on(&connection, screen)
}

pub(super) fn paste(target: Window) -> Result<(), String> {
    activate_and_confirm(target)?;
    simulate_ctrl_v()
}

fn activate_and_confirm(target: Window) -> Result<(), String> {
    let (connection, screen) = RustConnection::connect(None).map_err(|error| error.to_string())?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| "X11 screen 不存在".to_string())?
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
        .map_err(|error| error.to_string())?;
    connection.flush().map_err(|error| error.to_string())?;

    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if active_window_on(&connection, screen).ok() == Some(target) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err("X11 窗口管理器未恢复原活动窗口，已取消按键注入".to_string())
}

fn simulate_ctrl_v() -> Result<(), String> {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard, Settings,
    };

    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|error| format!("初始化 enigo 失败: {error}"))?;
    enigo
        .key(Key::Control, Press)
        .map_err(|error| format!("按下 Control 失败: {error}"))?;
    let click = enigo.key(Key::Unicode('v'), Click);
    let release = enigo.key(Key::Control, Release);
    click.map_err(|error| format!("按下 V 失败: {error}"))?;
    release.map_err(|error| format!("释放 Control 失败: {error}"))?;
    Ok(())
}

fn active_window_on(connection: &RustConnection, screen: usize) -> Result<Window, String> {
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| "X11 screen 不存在".to_string())?
        .root;
    let active_atom = atom(connection, b"_NET_ACTIVE_WINDOW")?;
    let reply = connection
        .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?;
    reply
        .value32()
        .and_then(|mut values| values.next())
        .filter(|window| *window != 0)
        .ok_or_else(|| "X11 活动窗口为空".to_string())
}

fn atom(connection: &RustConnection, name: &[u8]) -> Result<u32, String> {
    let atom = connection
        .intern_atom(false, name)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?
        .atom;
    if atom == 0 {
        Err(format!(
            "X11 atom 不存在: {}",
            String::from_utf8_lossy(name)
        ))
    } else {
        Ok(atom)
    }
}
