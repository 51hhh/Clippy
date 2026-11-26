#[cfg(all(
    feature = "native-recording-audio",
    any(target_os = "linux", target_os = "windows")
))]
use super::{send, Message};
use crate::{locale::tr, RecordingLibrary};
use anyhow::{Context, Result};
#[cfg(all(
    feature = "native-recording-audio",
    any(target_os = "linux", target_os = "windows")
))]
use clippy_media::recording::audio_devices::{
    NativeRecordingAudioDevice, RecordingAudioDeviceKind,
};
#[cfg(all(feature = "native-recording-audio", target_os = "linux"))]
use clippy_media::recording::platform::linux_audio::{
    enumerate_recording_audio_devices, LinuxPipeWireAudioSourcePlan as SourcePlan,
};
#[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
use clippy_media::recording::platform::macos_audio::{
    system_audio_available, MacSystemAudioPlan as SourcePlan,
};
#[cfg(all(feature = "native-recording-audio", target_os = "windows"))]
use clippy_media::recording::platform::windows_audio::{
    enumerate_recording_audio_devices, WindowsWasapiAudioSourcePlan as SourcePlan,
};
#[cfg(feature = "native-recording-audio")]
use clippy_platform::capture::Region;
use slint::{ModelRc, VecModel};
use std::io;

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) enum AudioTrack {
    Silent,
    #[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
    MacSystem,
    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    System(Option<String>),
    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    Microphone(Option<String>),
}

impl AudioTrack {
    #[cfg(feature = "native-recording-audio")]
    pub(super) fn into_plan(self, _region: &Region) -> SourcePlan {
        match self {
            #[cfg(target_os = "macos")]
            Self::MacSystem => SourcePlan::for_region(_region),
            #[cfg(target_os = "linux")]
            Self::System(id) => SourcePlan::system_audio(id),
            #[cfg(target_os = "windows")]
            Self::System(id) => SourcePlan::system_loopback(id),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            Self::Microphone(id) => SourcePlan::microphone(id),
            Self::Silent => unreachable!("无声录屏不打开音频源"),
        }
    }
}

struct Choice {
    track: AudioTrack,
    label: String,
}

impl Choice {
    fn new(track: AudioTrack, label: impl Into<String>) -> Self {
        Self {
            track,
            label: label.into(),
        }
    }
}

pub(super) struct AudioOptions {
    choices: Vec<Choice>,
    selection: AudioTrack,
    status: &'static str,
    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    refreshing: bool,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            choices: default_choices(),
            selection: AudioTrack::Silent,
            status: "",
            #[cfg(all(
                feature = "native-recording-audio",
                any(target_os = "linux", target_os = "windows")
            ))]
            refreshing: false,
        }
    }
}

impl AudioOptions {
    pub(super) fn remember(&mut self, value: &str) {
        if let Some(choice) = self
            .choices
            .iter()
            .find(|choice| tr(&choice.label) == value)
        {
            self.selection = choice.track.clone();
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.choices
            .iter()
            .position(|choice| choice.track == self.selection)
    }

    pub(super) fn selected(&self) -> Result<AudioTrack> {
        self.selected_index()
            .map(|_| self.selection.clone())
            .context(tr(
                "The selected audio device is unavailable. Choose an audio source in Recordings.",
            ))
    }

    pub(super) fn show(&self, window: &RecordingLibrary) {
        let mut labels = self
            .choices
            .iter()
            .map(|choice| tr(&choice.label).into())
            .collect::<Vec<_>>();
        let value = if let Some(index) = self.selected_index() {
            tr(&self.choices[index].label)
        } else {
            let unavailable = tr("Selected device unavailable");
            labels.push(unavailable.into());
            unavailable
        };
        window.set_audio_options(ModelRc::new(VecModel::from(labels)));
        window.set_audio_value(value.into());
        self.show_status(window);
    }

    pub(super) fn show_status(&self, window: &RecordingLibrary) {
        window.set_audio_status(tr(self.status).into());
    }

    pub(super) fn refresh(&mut self) -> io::Result<()> {
        #[cfg(all(
            feature = "native-recording-audio",
            any(target_os = "linux", target_os = "windows")
        ))]
        if !self.refreshing {
            std::thread::Builder::new()
                .name("clippy-recording-devices".into())
                .spawn(|| {
                    let result = enumerate_recording_audio_devices().map_err(anyhow::Error::from);
                    send(Message::AudioDevices(result));
                })?;
            self.refreshing = true;
            self.status = "Finding audio devices…";
        }
        Ok(())
    }

    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    pub(super) fn updated(&mut self, devices: Result<Vec<NativeRecordingAudioDevice>>) {
        self.refreshing = false;
        match devices {
            Ok(devices) => {
                self.choices = catalog_choices(devices);
                self.status = "";
            }
            Err(error) => {
                eprintln!("Recording devices: {error}");
                self.status = "Could not refresh audio devices";
            }
        }
    }
}

#[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
fn default_choices() -> Vec<Choice> {
    let mut choices = vec![Choice::new(AudioTrack::Silent, "No audio")];
    if system_audio_available() {
        choices.push(Choice::new(AudioTrack::MacSystem, "Default system audio"));
    }
    choices
}

#[cfg(not(all(feature = "native-recording-audio", target_os = "macos")))]
fn default_choices() -> Vec<Choice> {
    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    {
        vec![
            Choice::new(AudioTrack::Silent, "No audio"),
            Choice::new(AudioTrack::System(None), "Default system audio"),
            Choice::new(AudioTrack::Microphone(None), "Default microphone"),
        ]
    }
    #[cfg(not(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    )))]
    {
        vec![Choice::new(AudioTrack::Silent, "No audio")]
    }
}

#[cfg(all(
    feature = "native-recording-audio",
    any(target_os = "linux", target_os = "windows")
))]
fn catalog_choices(devices: Vec<NativeRecordingAudioDevice>) -> Vec<Choice> {
    use std::collections::HashSet;

    let mut choices = default_choices();
    for (kind, heading) in [
        (RecordingAudioDeviceKind::SystemAudio, "System audio"),
        (RecordingAudioDeviceKind::Microphone, "Microphone"),
    ] {
        let mut same_kind = devices
            .iter()
            .filter(|device| device.kind == kind && !device.native_id.is_empty())
            .collect::<Vec<_>>();
        same_kind.sort_by(|first, second| {
            second
                .is_default
                .cmp(&first.is_default)
                .then_with(|| first.label.cmp(&second.label))
                .then_with(|| first.native_id.cmp(&second.native_id))
        });
        let mut identities = HashSet::new();
        let mut labels = HashSet::new();
        for device in same_kind
            .into_iter()
            .filter(|device| identities.insert(device.native_id.as_str()))
            .take(64)
        {
            let mut label: String = device
                .label
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .filter(|character| !character.is_control())
                .take(160)
                .collect();
            if label.is_empty() {
                label = "Unnamed device".into();
            }
            let stem = label.clone();
            let mut suffix = 2;
            while !labels.insert(label.clone()) {
                label = format!("{stem} ({suffix})");
                suffix += 1;
            }
            let track = match kind {
                RecordingAudioDeviceKind::SystemAudio => {
                    AudioTrack::System(Some(device.native_id.clone()))
                }
                RecordingAudioDeviceKind::Microphone => {
                    AudioTrack::Microphone(Some(device.native_id.clone()))
                }
            };
            choices.push(Choice::new(track, format!("{heading} · {label}")));
        }
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_a_silent_recording() {
        assert!(matches!(
            AudioOptions::default().selected(),
            Ok(AudioTrack::Silent)
        ));
    }

    #[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
    #[test]
    fn macos_system_audio_choice_follows_the_runtime() {
        let mut options = AudioOptions::default();
        options.remember(tr("Default system audio"));
        assert_eq!(
            matches!(options.selected(), Ok(AudioTrack::MacSystem)),
            system_audio_available()
        );
    }

    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn refresh_keeps_device_identity_and_rejects_a_removed_selection() {
        let device = |id: &str| NativeRecordingAudioDevice {
            kind: RecordingAudioDeviceKind::Microphone,
            native_id: id.into(),
            label: "Desk microphone".into(),
            is_default: false,
        };
        let mut options = AudioOptions::default();
        options.updated(Ok(vec![device("private-b"), device("private-a")]));
        assert_eq!(options.choices[3].label, "Microphone · Desk microphone");
        assert_eq!(options.choices[4].label, "Microphone · Desk microphone (2)");
        options.remember("Microphone · Desk microphone (2)");
        options.updated(Ok(vec![device("private-b")]));
        assert!(
            matches!(options.selected(), Ok(AudioTrack::Microphone(Some(id))) if id == "private-b")
        );
        options.updated(Ok(vec![device("private-a")]));
        assert!(options.selected().is_err());
        options.remember("Default microphone");
        assert!(matches!(
            options.selected(),
            Ok(AudioTrack::Microphone(None))
        ));
    }
}
