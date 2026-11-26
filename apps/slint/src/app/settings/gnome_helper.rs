use super::{send, use_app, App, Message};
use crate::locale::tr;
use crate::{AppWindow, GnomeHelperAction, GnomeHelperView};
use anyhow::Result;
use clippy_platform::{
    gnome_extension::{self, Action, State, Status},
    identity::DistributionChannel,
};
use std::thread;

pub(super) fn connect(window: &AppWindow) {
    window.on_gnome_helper_action(|action| {
        use_app(|app| request(app, action));
    });
}

fn request(app: &mut App, action: GnomeHelperAction) {
    let window = app.window.as_ref().expect("扩展操作来自设置页");
    window.set_gnome_helper_busy(true);
    if app.gnome_helper_busy {
        return;
    }
    app.gnome_helper_busy = true;
    let action = match action {
        GnomeHelperAction::Refresh => Action::Refresh,
        GnomeHelperAction::Install => Action::Install,
        GnomeHelperAction::Enable => Action::Enable,
        GnomeHelperAction::Disable => Action::Disable,
        GnomeHelperAction::Remove => Action::Remove,
    };
    if let Err(error) = thread::Builder::new()
        .name("clippy-gnome-helper".into())
        .spawn(move || send(Message::GnomeHelper(gnome_extension::manage(action))))
    {
        completed(app, Err(error.into()));
    }
}

pub(super) fn completed(app: &mut App, result: Result<Status>) {
    app.gnome_helper_busy = false;
    if app.identity.channel == DistributionChannel::Qa {
        match &result {
            Ok(status) => eprintln!("gnome_helper_state={:?}", status.state),
            Err(error) => eprintln!("gnome_helper_state=Error {error}"),
        }
    }
    if let Some(window) = app
        .window
        .as_ref()
        .filter(|window| window.get_show_settings())
    {
        window.set_gnome_helper_busy(false);
        window.set_gnome_helper(match result {
            Ok(status) => view(status),
            Err(error) => GnomeHelperView {
                title: tr("GNOME helper unavailable").into(),
                detail: error.to_string().into(),
                ..Default::default()
            },
        });
    }
}

fn view(status: Status) -> GnomeHelperView {
    use GnomeHelperAction::{Disable, Enable, Install, Refresh};

    let (title, detail, action, label) = match status.state {
        State::NotInstalled => (
            "Helper not installed",
            "Install the bundled helper to select open windows during a screenshot.",
            Install,
            "Install and Enable",
        ),
        State::Disabled => (
            "Helper disabled",
            "Enable it to bring back window picking.",
            Enable,
            "Enable",
        ),
        State::AwaitingLogin => (
            "Sign out to finish setup",
            "GNOME will load the installed helper the next time you sign in.",
            Refresh,
            "",
        ),
        State::Ready => (
            "Window picking ready",
            "Hover over an open window during capture to highlight it.",
            Disable,
            "Disable",
        ),
        State::UpdateAvailable => (
            "Helper update available",
            "Install the bundled version. GNOME loads updated code when you sign in.",
            Install,
            "Update and Enable",
        ),
        State::NeedsRepair => (
            "Helper needs repair",
            "Restore the bundled files and private token.",
            Install,
            "Repair and Enable",
        ),
        State::UserExtensionsOff => (
            "GNOME user extensions are off",
            "Turn on User Extensions in the GNOME Extensions app.",
            Refresh,
            "",
        ),
        State::Unavailable => (
            "GNOME has not loaded the helper",
            "Refresh its status or sign out and back in to reload it.",
            Refresh,
            "",
        ),
        State::ManagedElsewhere => (
            "Helper managed outside this copy of Clippy",
            "Manage this installation in the GNOME Extensions app.",
            Refresh,
            "",
        ),
        State::UnsupportedShell => (
            "Helper unavailable on this Shell version",
            "The bundled helper supports the GNOME versions listed in its manifest.",
            Refresh,
            "",
        ),
    };
    let title = tr(title);
    let title = if let Some(version) = status.version {
        format!("{title} · v{version}").into()
    } else {
        title.into()
    };
    GnomeHelperView {
        title,
        detail: tr(detail).into(),
        action,
        action_label: tr(label).into(),
        removable: status.removable,
    }
}
