#[cfg(feature = "recording-playback-gstreamer")]
mod gstreamer;
#[cfg(all(
    any(target_os = "linux", target_os = "windows"),
    feature = "recording-playback-native-stream"
))]
mod native;

#[cfg(feature = "recording-playback-gstreamer")]
pub use gstreamer::RecordingPlayer;
#[cfg(all(
    any(target_os = "linux", target_os = "windows"),
    feature = "recording-playback-native-stream"
))]
pub use native::NativeRecordingPlayer;

pub struct PlaybackUpdate {
    pub duration_ms: u64,
    pub position_ms: u64,
    pub frame: Option<image::RgbaImage>,
    pub ended: bool,
}
