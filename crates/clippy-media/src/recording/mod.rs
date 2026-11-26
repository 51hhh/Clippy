pub mod audio;
pub mod audio_devices;
pub mod audio_mixer;
pub mod audio_worker;
#[cfg(feature = "recording-opus-webm")]
pub mod av_bridge_join;
#[cfg(feature = "recording-opus-webm")]
pub mod av_encoder_worker;
#[cfg(feature = "recording-opus-webm")]
pub mod av_recovery;
#[cfg(feature = "recording-opus-webm")]
pub mod av_segmenting;
#[cfg(feature = "recording-opus-webm")]
pub mod av_session;
pub mod av_timeline;
pub mod clock;
pub mod encoder_worker;
pub mod frame;
pub mod manifest;
pub mod mux;
#[cfg(feature = "recording-playback-native")]
pub mod native_playback;
pub mod pipeline;
pub mod platform;
#[cfg(any(
    feature = "recording-playback-gstreamer",
    all(target_os = "linux", feature = "recording-playback-native-stream")
))]
pub mod playback;
pub mod segmenting;
pub mod session;
#[cfg(test)]
mod test_artifacts;
#[cfg(feature = "recording-vp9-prototype")]
pub mod thumbnail;
pub mod timeline;
pub mod worker;
pub mod worker_start;

use std::{fmt, io, path::Path};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecoverySummary {
    pub scanned_sessions: usize,
    pub interrupted_sessions: usize,
    pub recoverable_segments: usize,
    pub rejected_sessions: usize,
    pub session_limit_reached: bool,
}

impl RecoverySummary {
    pub fn has_activity(self) -> bool {
        self.interrupted_sessions > 0 || self.rejected_sessions > 0 || self.session_limit_reached
    }
}

impl fmt::Display for RecoverySummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "扫描 {} 个会话，中断 {} 个，可恢复 {} 个分段，拒绝 {} 个{}",
            self.scanned_sessions,
            self.interrupted_sessions,
            self.recoverable_segments,
            self.rejected_sessions,
            if self.session_limit_reached {
                "，已达到扫描上限"
            } else {
                ""
            }
        )
    }
}

pub fn recover_interrupted_sessions(app_data_dir: &Path) -> io::Result<RecoverySummary> {
    manifest::recover_interrupted_sessions(app_data_dir)
}
