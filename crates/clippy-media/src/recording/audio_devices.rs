#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingAudioDeviceKind {
    SystemAudio,
    Microphone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRecordingAudioDevice {
    pub kind: RecordingAudioDeviceKind,
    pub native_id: String,
    pub label: String,
    pub is_default: bool,
}
