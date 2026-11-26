use super::{ControlWindow, Direction};
use crate::capture::DesktopRect;
use anyhow::{ensure, Result};
use x11rb::{
    connection::{Connection, RequestConnection},
    errors::ReplyError,
    protocol::{
        xproto::{
            Atom, AtomEnum, ConnectionExt as _, KeyButMask, MapState, QueryPointerReply, Window,
            BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT,
        },
        xtest::{ConnectionExt as _, X11_EXTENSION_NAME},
        ErrorKind,
    },
    rust_connection::RustConnection,
    CURRENT_TIME, NONE,
};

type Point = (i16, i16);

pub(super) struct Target {
    connection: RustConnection,
    root: Window,
    anchor: Point,
    window: Window,
    client: Option<(Window, u32)>,
    pid_atom: Atom,
    original_pointer: Option<Point>,
}

impl Target {
    pub(super) fn open(region: DesktopRect) -> Result<Option<Self>> {
        let (connection, screen) = x11rb::connect(None)?;
        if connection
            .extension_information(X11_EXTENSION_NAME)?
            .is_none()
        {
            return Ok(None);
        }
        let root = connection.setup().roots[screen].root;
        let anchor = (
            i16::try_from(i64::from(region.x) + i64::from(region.width / 2))?,
            i16::try_from(i64::from(region.y) + i64::from(region.height / 2))?,
        );
        let window = child_at(&connection, root, root, anchor)?;
        if window == NONE {
            return Ok(None);
        }
        let pid_atom = connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let mut client = None;
        let mut child = window;
        while child != NONE {
            if let Some(pid) = window_pid(&connection, child, pid_atom)? {
                if pid == std::process::id() {
                    return Ok(None);
                }
                client = Some((child, pid));
                break;
            }
            // 重父窗口管理器的外框通常没有 _NET_WM_PID，沿同一个命中点找到客户窗口。
            child = child_at(&connection, root, child, anchor)?;
        }
        Ok(Some(Self {
            connection,
            root,
            anchor,
            window,
            client,
            pid_atom,
            original_pointer: None,
        }))
    }

    fn check_client(&self) -> Result<()> {
        if let Some((client, pid)) = self.client {
            ensure!(
                window_pid(&self.connection, client, self.pid_atom)? == Some(pid),
                "Scroll target changed"
            );
        }
        Ok(())
    }

    fn pointer(&self) -> Result<QueryPointerReply> {
        Ok(self.connection.query_pointer(self.root)?.reply()?)
    }

    fn move_pointer(&self, (x, y): Point) -> Result<()> {
        self.connection
            .warp_pointer(NONE, self.root, 0, 0, 0, 0, x, y)?
            .check()?;
        Ok(())
    }
}

impl super::Target for Target {
    fn control_is_hidden(&self, ControlWindow(window): ControlWindow) -> Result<bool> {
        match self.connection.get_window_attributes(window)?.reply() {
            Ok(reply) => Ok(reply.map_state != MapState::VIEWABLE),
            Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    fn position_pointer(&mut self) -> Result<()> {
        ensure!(
            child_at(&self.connection, self.root, self.root, self.anchor)? == self.window,
            "Scroll target changed"
        );
        self.check_client()?;
        let pointer = self.pointer()?;
        ensure!(pointer.same_screen, "Pointer moved to another screen");
        ensure_unmodified(&pointer)?;
        self.original_pointer = Some((pointer.root_x, pointer.root_y));
        self.move_pointer(self.anchor)?;
        self.check()
    }

    fn check(&self) -> Result<()> {
        let pointer = self.pointer()?;
        ensure!(
            pointer.same_screen && (pointer.root_x, pointer.root_y) == self.anchor,
            "Pointer moved. Automatic scrolling stopped"
        );
        ensure!(pointer.child == self.window, "Scroll target changed");
        ensure_unmodified(&pointer)
    }

    fn step(&self, direction: Direction) -> Result<()> {
        self.check_client()?;
        self.check()?;
        let button = match direction {
            Direction::Up => 4,
            Direction::Down => 5,
            Direction::Left => 6,
            Direction::Right => 7,
        };
        let pressed = self.connection.xtest_fake_input(
            BUTTON_PRESS_EVENT,
            button,
            CURRENT_TIME,
            self.root,
            0,
            0,
            0,
        )?;
        let released = self.connection.xtest_fake_input(
            BUTTON_RELEASE_EVENT,
            button,
            CURRENT_TIME,
            self.root,
            0,
            0,
            0,
        )?;
        pressed.check()?;
        released.check()?;
        Ok(())
    }

    fn finish(&mut self, restore_pointer: bool) -> Result<()> {
        if let Some(original) = self.original_pointer.take().filter(|_| restore_pointer) {
            let pointer = self.pointer()?;
            if pointer.same_screen && (pointer.root_x, pointer.root_y) == self.anchor {
                self.move_pointer(original)?;
            }
        }
        Ok(())
    }
}

fn child_at(
    connection: &RustConnection,
    root: Window,
    window: Window,
    (x, y): Point,
) -> Result<Window> {
    Ok(connection
        .translate_coordinates(root, window, x, y)?
        .reply()?
        .child)
}

fn window_pid(connection: &RustConnection, window: Window, atom: Atom) -> Result<Option<u32>> {
    let property = connection
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?;
    Ok(property.value32().and_then(|mut values| values.next()))
}

fn ensure_unmodified(pointer: &QueryPointerReply) -> Result<()> {
    let modifiers = KeyButMask::SHIFT | KeyButMask::CONTROL | KeyButMask::MOD1 | KeyButMask::MOD4;
    let buttons = KeyButMask::BUTTON1
        | KeyButMask::BUTTON2
        | KeyButMask::BUTTON3
        | KeyButMask::BUTTON4
        | KeyButMask::BUTTON5;
    ensure!(
        u16::from(pointer.mask & (modifiers | buttons)) == 0,
        "Release keys and mouse buttons to use automatic scrolling"
    );
    Ok(())
}
