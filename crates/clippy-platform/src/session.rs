pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland")
            || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()))
}

pub fn is_gnome() -> bool {
    cfg!(target_os = "linux")
        && std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|value| {
            value
                .split(':')
                .any(|part| part.eq_ignore_ascii_case("GNOME"))
        })
}
