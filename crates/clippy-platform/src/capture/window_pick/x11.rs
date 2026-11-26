use super::{DesktopRect, DesktopWindow};
use anyhow::Result;
use x11rb::{
    connection::Connection,
    protocol::xproto::{
        Atom, AtomEnum, ConnectionExt, GetPropertyReply, MapState, Window, WindowClass,
    },
    rust_connection::RustConnection,
    NONE,
};

struct Atoms {
    stacking: Atom,
    current_desktop: Atom,
    desktop: Atom,
    pid: Atom,
    name: Atom,
    window_type: Atom,
    desktop_type: Atom,
    dock_type: Atom,
    state: Atom,
    hidden: Atom,
    gtk_extents: Atom,
    net_extents: Atom,
}

impl Atoms {
    fn read(connection: &RustConnection) -> Result<Self> {
        let requests = [
            "_NET_CLIENT_LIST_STACKING",
            "_NET_CURRENT_DESKTOP",
            "_NET_WM_DESKTOP",
            "_NET_WM_PID",
            "_NET_WM_NAME",
            "_NET_WM_WINDOW_TYPE",
            "_NET_WM_WINDOW_TYPE_DESKTOP",
            "_NET_WM_WINDOW_TYPE_DOCK",
            "_NET_WM_STATE",
            "_NET_WM_STATE_HIDDEN",
            "_GTK_FRAME_EXTENTS",
            "_NET_FRAME_EXTENTS",
        ]
        .map(|name| connection.intern_atom(false, name.as_bytes()));
        let [stacking, current_desktop, desktop, pid, name, window_type, desktop_type, dock_type, state, hidden, gtk_extents, net_extents] =
            requests.map(|request| -> Result<_> { Ok(request?.reply()?) });
        Ok(Self {
            stacking: stacking?.atom,
            current_desktop: current_desktop?.atom,
            desktop: desktop?.atom,
            pid: pid?.atom,
            name: name?.atom,
            window_type: window_type?.atom,
            desktop_type: desktop_type?.atom,
            dock_type: dock_type?.atom,
            state: state?.atom,
            hidden: hidden?.atom,
            gtk_extents: gtk_extents?.atom,
            net_extents: net_extents?.atom,
        })
    }
}

pub(super) fn windows() -> Result<Vec<DesktopWindow>> {
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let atoms = Atoms::read(&connection)?;
    let stack = connection
        .get_property(false, root, atoms.stacking, AtomEnum::WINDOW, 0, 4096)?
        .reply()?;
    // EWMH 与裸 X11 的 QueryTree 都从底层到顶层列出窗口。
    let clients: Vec<_> = if let Some(list) = stack.value32() {
        list.collect()
    } else {
        connection.query_tree(root)?.reply()?.children
    };
    let desktop = cardinal(
        &connection
            .get_property(false, root, atoms.current_desktop, AtomEnum::CARDINAL, 0, 1)?
            .reply()?,
    );
    Ok(clients
        .into_iter()
        .rev()
        .filter_map(|window| snapshot(&connection, root, window, &atoms, desktop))
        .collect())
}

fn snapshot(
    connection: &RustConnection,
    root: Window,
    window: Window,
    atoms: &Atoms,
    active: Option<u32>,
) -> Option<DesktopWindow> {
    let attrs = connection.get_window_attributes(window).ok()?;
    let geo = connection.get_geometry(window).ok()?;
    let origin = connection.translate_coordinates(window, root, 0, 0).ok()?;
    let property = |atom: Atom, size| {
        connection
            .get_property(false, window, atom, NONE, 0, size)
            .ok()
    };
    let desktop = property(atoms.desktop, 1)?;
    let state = property(atoms.state, 32)?;
    let kind = property(atoms.window_type, 8)?;
    let gtk = property(atoms.gtk_extents, 4)?;
    let net = property(atoms.net_extents, 4)?;
    let title = property(atoms.name, 128)?;
    let wm_name = property(AtomEnum::WM_NAME.into(), 128)?;
    let pid = property(atoms.pid, 1)?;
    let attrs = attrs.reply().ok()?;
    if attrs.map_state != MapState::VIEWABLE
        || attrs.class != WindowClass::INPUT_OUTPUT
        || attrs.override_redirect
    {
        return None;
    }
    if let (Some(active), Some(window_desktop)) = (active, cardinal(&desktop.reply().ok()?)) {
        if window_desktop != active && window_desktop != u32::MAX {
            return None;
        }
    }
    if state
        .reply()
        .ok()?
        .value32()
        .is_some_and(|mut state| state.any(|atom| atom == atoms.hidden))
        || kind.reply().ok()?.value32().is_some_and(|mut kinds| {
            kinds.any(|atom| atom == atoms.desktop_type || atom == atoms.dock_type)
        })
    {
        return None;
    }
    let geo = geo.reply().ok()?;
    let origin = origin.reply().ok()?;
    let rect = DesktopRect {
        x: i32::from(origin.dst_x),
        y: i32::from(origin.dst_y),
        width: u32::from(geo.width),
        height: u32::from(geo.height),
    };
    let border = u32::from(geo.border_width);
    let geometry = if let Some(extents) = extents(&gtk.reply().ok()?) {
        // GTK 的边距是客户区里的透明阴影，和窗口管理器在客户区之外的边框方向相反。
        frame(rect, extents, true).unwrap_or(rect)
    } else {
        frame(
            rect,
            extents(&net.reply().ok()?).unwrap_or([border; 4]),
            false,
        )?
    };
    let title = title.reply().ok()?.value;
    let title = if title.is_empty() {
        wm_name.reply().ok()?.value
    } else {
        title
    };
    Some(DesktopWindow {
        geometry,
        pid: cardinal(&pid.reply().ok()?).unwrap_or(0),
        title: String::from_utf8_lossy(&title).into_owned(),
    })
}

fn cardinal(property: &GetPropertyReply) -> Option<u32> {
    property.value32()?.next()
}

fn extents(property: &GetPropertyReply) -> Option<[u32; 4]> {
    let mut values = property.value32()?;
    Some([
        values.next()?,
        values.next()?,
        values.next()?,
        values.next()?,
    ])
}

fn frame(
    client: DesktopRect,
    [left, right, top, bottom]: [u32; 4],
    inset: bool,
) -> Option<DesktopRect> {
    let sign = if inset { 1 } else { -1 };
    let width = i64::from(client.width) - sign * (i64::from(left) + i64::from(right));
    let height = i64::from(client.height) - sign * (i64::from(top) + i64::from(bottom));
    Some(DesktopRect {
        x: (i64::from(client.x) + sign * i64::from(left))
            .try_into()
            .ok()?,
        y: (i64::from(client.y) + sign * i64::from(top))
            .try_into()
            .ok()?,
        width: width.try_into().ok()?,
        height: height.try_into().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_manager_frames_and_trims_gtk_shadows_in_their_own_directions() {
        let client = DesktopRect {
            x: -80,
            y: 40,
            width: 400,
            height: 200,
        };
        assert_eq!(
            frame(client, [4, 6, 24, 5], false),
            Some(DesktopRect {
                x: -84,
                y: 16,
                width: 410,
                height: 229
            }),
        );
        assert_eq!(
            frame(client, [4, 6, 24, 5], true),
            Some(DesktopRect {
                x: -76,
                y: 64,
                width: 390,
                height: 171
            }),
        );
    }
}
