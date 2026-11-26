#[cfg(feature = "native-recording-audio")]
use super::{send, Message};
use crate::{locale::tr, RecordingLibrary};
use anyhow::{Context, Result};
#[cfg(feature = "native-recording-audio")]
use clippy_media::recording::audio_devices::{
    NativeRecordingAudioDevice, RecordingAudioDeviceKind,
};
#[cfg(feature = "native-recording-audio")]
use clippy_media::recording::audio_mixer::OUTPUT_CHANNELS;
#[cfg(all(feature = "native-recording-audio", target_os = "linux"))]
use clippy_media::recording::platform::linux_audio::{
    enumerate_recording_audio_devices, LinuxPipeWireAudioSourcePlan as SourcePlan,
};
#[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
use clippy_media::recording::platform::macos_audio::{
    enumerate_recording_audio_devices, microphone_available, request_microphone_permission,
    system_audio_available, MacAudioPlan as SourcePlan,
};
#[cfg(all(feature = "native-recording-audio", target_os = "windows"))]
use clippy_media::recording::platform::windows_audio::{
    enumerate_recording_audio_devices, WindowsWasapiAudioSourcePlan as SourcePlan,
};
#[cfg(feature = "native-recording-audio")]
use clippy_platform::capture::Region;
#[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
use clippy_platform::capture::{macos_audio::AudioInput, RegionCancellation};
use slint::{ModelRc, SharedString, VecModel};
use std::io;

#[cfg(feature = "native-recording-audio")]
const MIXED_LABEL: &str = "System audio + microphone";
const UNAVAILABLE_LABEL: &str = "Selected device unavailable";
const UNAVAILABLE_ERROR: &str =
    "The selected audio device is unavailable. Choose an audio source in Recordings.";

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) enum AudioTrack {
    Silent,
    #[cfg(feature = "native-recording-audio")]
    System(Option<String>),
    #[cfg(feature = "native-recording-audio")]
    Microphone(Option<String>),
    #[cfg(feature = "native-recording-audio")]
    Mixed {
        system: Option<String>,
        microphone: Option<String>,
    },
}

impl AudioTrack {
    #[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
    pub(super) fn prepare(&self, cancellation: &RegionCancellation) -> Result<()> {
        if matches!(self, Self::Microphone(_) | Self::Mixed { .. }) {
            request_microphone_permission(cancellation)?;
        }
        Ok(())
    }

    #[cfg(feature = "native-recording-audio")]
    pub(super) fn into_plan(self, region: &Region) -> AudioPlan {
        match self {
            Self::System(id) => AudioPlan::Single(system_plan(region, id)),
            Self::Microphone(id) => AudioPlan::Single(microphone_plan(region, id)),
            Self::Mixed { system, microphone } => AudioPlan::Mixed {
                system: system_plan(region, system),
                microphone: microphone_plan(region, microphone),
            },
            Self::Silent => unreachable!("无声录屏不打开音频源"),
        }
    }
}

#[cfg(feature = "native-recording-audio")]
pub(super) enum AudioPlan {
    Single(SourcePlan),
    Mixed {
        system: SourcePlan,
        microphone: SourcePlan,
    },
}

#[cfg(feature = "native-recording-audio")]
impl AudioPlan {
    pub(super) fn channels(&self) -> u16 {
        match self {
            Self::Single(plan) => plan.channels(),
            Self::Mixed { .. } => OUTPUT_CHANNELS,
        }
    }
}

#[cfg(feature = "native-recording-audio")]
fn system_plan(_region: &Region, _id: Option<String>) -> SourcePlan {
    #[cfg(target_os = "macos")]
    {
        SourcePlan::for_region(_region, AudioInput::System)
    }
    #[cfg(target_os = "linux")]
    {
        SourcePlan::system_audio(_id)
    }
    #[cfg(target_os = "windows")]
    {
        SourcePlan::system_loopback(_id)
    }
}

#[cfg(feature = "native-recording-audio")]
fn microphone_plan(_region: &Region, id: Option<String>) -> SourcePlan {
    #[cfg(target_os = "macos")]
    {
        SourcePlan::for_region(_region, AudioInput::Microphone(id))
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        SourcePlan::microphone(id)
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
    #[cfg(feature = "native-recording-audio")]
    refreshing: bool,
    #[cfg(feature = "native-recording-audio")]
    mixed_system: Option<String>,
    #[cfg(feature = "native-recording-audio")]
    mixed_microphone: Option<String>,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            choices: default_choices(),
            selection: AudioTrack::Silent,
            status: "",
            #[cfg(feature = "native-recording-audio")]
            refreshing: false,
            #[cfg(feature = "native-recording-audio")]
            mixed_system: None,
            #[cfg(feature = "native-recording-audio")]
            mixed_microphone: None,
        }
    }
}

impl AudioOptions {
    pub(super) fn remember(&mut self, value: &str) {
        if let Some(track) = self.track_for_label(value) {
            self.selection = track;
        }
    }

    #[cfg(feature = "native-recording-audio")]
    pub(super) fn remember_mixed(&mut self, system: &str, microphone: &str) {
        if let Some(AudioTrack::System(id)) = self.track_for_label(system) {
            self.mixed_system = id;
        }
        if let Some(AudioTrack::Microphone(id)) = self.track_for_label(microphone) {
            self.mixed_microphone = id;
        }
    }

    fn track_for_label(&self, value: &str) -> Option<AudioTrack> {
        self.choices
            .iter()
            .find(|choice| tr(&choice.label) == value)
            .map(|choice| choice.track.clone())
    }

    pub(super) fn selected(&self) -> Result<AudioTrack> {
        let available =
            |track: &AudioTrack| self.choices.iter().any(|choice| &choice.track == track);
        available(&self.selection)
            .then_some(())
            .context(tr(UNAVAILABLE_ERROR))?;
        #[cfg(feature = "native-recording-audio")]
        if matches!(self.selection, AudioTrack::Mixed { .. }) {
            let system = self.mixed_system.clone();
            let microphone = self.mixed_microphone.clone();
            return (available(&AudioTrack::System(system.clone()))
                && available(&AudioTrack::Microphone(microphone.clone())))
            .then_some(AudioTrack::Mixed { system, microphone })
            .context(tr(UNAVAILABLE_ERROR));
        }
        Ok(self.selection.clone())
    }

    pub(super) fn show(&self, window: &RecordingLibrary) {
        let (options, value) = present(self.choices.iter(), &self.selection);
        window.set_audio_options(options);
        window.set_audio_value(value);
        #[cfg(feature = "native-recording-audio")]
        if self
            .choices
            .iter()
            .any(|choice| matches!(choice.track, AudioTrack::Mixed { .. }))
        {
            let (options, value) = present(
                self.choices
                    .iter()
                    .filter(|choice| matches!(choice.track, AudioTrack::System(_))),
                &AudioTrack::System(self.mixed_system.clone()),
            );
            window.set_mixed_system_options(options);
            window.set_mixed_system_value(value);
            let (options, value) = present(
                self.choices
                    .iter()
                    .filter(|choice| matches!(choice.track, AudioTrack::Microphone(_))),
                &AudioTrack::Microphone(self.mixed_microphone.clone()),
            );
            window.set_mixed_microphone_options(options);
            window.set_mixed_microphone_value(value);
            window.set_mixed_audio_label(tr(MIXED_LABEL).into());
        }
        self.show_status(window);
    }

    pub(super) fn show_status(&self, window: &RecordingLibrary) {
        window.set_audio_status(tr(self.status).into());
    }

    pub(super) fn refresh(&mut self) -> io::Result<()> {
        #[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
        if !microphone_available() {
            return Ok(());
        }
        #[cfg(feature = "native-recording-audio")]
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

    #[cfg(feature = "native-recording-audio")]
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

fn present<'a>(
    choices: impl Iterator<Item = &'a Choice>,
    selected: &AudioTrack,
) -> (ModelRc<SharedString>, SharedString) {
    let mut labels = Vec::new();
    let mut value = None;
    for choice in choices {
        let label = SharedString::from(tr(&choice.label));
        if &choice.track == selected {
            value = Some(label.clone());
        }
        labels.push(label);
    }
    let value = value.unwrap_or_else(|| {
        let label = SharedString::from(tr(UNAVAILABLE_LABEL));
        labels.push(label.clone());
        label
    });
    (ModelRc::new(VecModel::from(labels)), value)
}

#[cfg(feature = "native-recording-audio")]
fn mixed_choice() -> Choice {
    Choice::new(
        AudioTrack::Mixed {
            system: None,
            microphone: None,
        },
        MIXED_LABEL,
    )
}

#[cfg(all(feature = "native-recording-audio", target_os = "macos"))]
fn default_choices() -> Vec<Choice> {
    let mut choices = vec![Choice::new(AudioTrack::Silent, "No audio")];
    if system_audio_available() {
        choices.push(Choice::new(
            AudioTrack::System(None),
            "Default system audio",
        ));
    }
    if microphone_available() {
        choices.push(Choice::new(
            AudioTrack::Microphone(None),
            "Default microphone",
        ));
        choices.push(mixed_choice());
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
            mixed_choice(),
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

#[cfg(feature = "native-recording-audio")]
fn catalog_choices(devices: Vec<NativeRecordingAudioDevice>) -> Vec<Choice> {
    use std::collections::HashSet;

    let mut choices = default_choices();
    for (kind, heading) in [
        #[cfg(any(target_os = "linux", target_os = "windows"))]
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
    fn macos_audio_choices_follow_the_runtime() {
        let mut options = AudioOptions::default();
        options.remember(tr("Default system audio"));
        assert_eq!(
            matches!(options.selected(), Ok(AudioTrack::System(None))),
            system_audio_available()
        );
        options.remember(tr("Default microphone"));
        assert_eq!(
            matches!(options.selected(), Ok(AudioTrack::Microphone(None))),
            microphone_available()
        );
        options.remember(tr(MIXED_LABEL));
        assert_eq!(
            matches!(options.selected(), Ok(AudioTrack::Mixed { .. })),
            microphone_available()
        );
    }

    #[cfg(feature = "native-recording-audio")]
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
        let devices = &options.choices[options.choices.len() - 2..];
        assert_eq!(devices[0].label, "Microphone · Desk microphone");
        assert_eq!(devices[1].label, "Microphone · Desk microphone (2)");
        options.remember("Microphone · Desk microphone (2)");
        options.updated(Ok(vec![device("private-b")]));
        assert!(
            matches!(options.selected(), Ok(AudioTrack::Microphone(Some(id))) if id == "private-b")
        );
        options.updated(Ok(vec![device("private-a")]));
        assert!(options.selected().is_err());
        #[cfg(target_os = "macos")]
        if !microphone_available() {
            return;
        }
        options.remember("Default microphone");
        assert!(matches!(
            options.selected(),
            Ok(AudioTrack::Microphone(None))
        ));
    }

    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn mixed_selection_recovers_an_independent_source_without_losing_the_other() {
        let speakers = NativeRecordingAudioDevice {
            kind: RecordingAudioDeviceKind::SystemAudio,
            native_id: "unshown-output-id".into(),
            label: "Dock speakers".into(),
            is_default: false,
        };
        let mut microphone = NativeRecordingAudioDevice {
            kind: RecordingAudioDeviceKind::Microphone,
            native_id: "unshown-input-id".into(),
            label: "Dock microphone".into(),
            is_default: true,
        };
        let mut options = AudioOptions::default();
        options.updated(Ok(vec![speakers.clone(), microphone.clone()]));
        options.remember(MIXED_LABEL);
        options.remember_mixed(
            "System audio · Dock speakers",
            "Microphone · Dock microphone",
        );
        microphone.label = "Renamed microphone".into();
        options.updated(Ok(vec![speakers, microphone.clone()]));
        assert!(
            matches!(options.selected(), Ok(AudioTrack::Mixed { system: Some(system), microphone: Some(microphone) })
            if system == "unshown-output-id" && microphone == "unshown-input-id")
        );

        options.updated(Ok(vec![microphone]));
        assert!(options.selected().is_err());
        options.remember_mixed("Default system audio", "Microphone · Renamed microphone");
        assert!(
            matches!(options.selected(), Ok(AudioTrack::Mixed { system: None, microphone: Some(microphone) })
            if microphone == "unshown-input-id")
        );
    }
}
