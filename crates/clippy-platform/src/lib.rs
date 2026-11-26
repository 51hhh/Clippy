pub mod capture;
pub mod clipboard;
pub mod file_dialog;
#[cfg(target_os = "linux")]
pub mod gnome_extension;
#[cfg(target_os = "linux")]
mod gnome_settings;
pub mod identity;
pub mod instance;
pub mod paste;
pub mod paths;
pub mod private_files;
pub mod recording_control;
pub mod scroll;
pub mod session;
pub mod shortcuts;
pub mod startup;
