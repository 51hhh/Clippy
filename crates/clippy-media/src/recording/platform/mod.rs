#[cfg(all(target_os = "linux", feature = "recording-linux-audio"))]
pub mod linux_audio;
#[cfg(any(test, all(target_os = "linux", feature = "recording-linux-audio")))]
#[cfg_attr(
    not(all(target_os = "linux", feature = "recording-linux-audio")),
    allow(dead_code)
)]
mod linux_audio_contract;
#[cfg(all(target_os = "macos", feature = "recording-macos-audio"))]
pub mod macos_audio;
#[cfg(all(target_os = "windows", feature = "recording-windows-audio"))]
pub mod windows_audio;
#[cfg(any(test, all(target_os = "windows", feature = "recording-windows-audio")))]
#[cfg_attr(
    not(all(target_os = "windows", feature = "recording-windows-audio")),
    allow(dead_code)
)]
mod windows_audio_contract;
