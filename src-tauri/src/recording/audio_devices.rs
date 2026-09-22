//! 录屏音频设备目录只向 WebView 暴露短期不透明 token；原生设备身份始终留在 Rust 侧。

use super::lifecycle::RecordingAudioMode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_DEVICES_PER_KIND: usize = 64;
const MAX_LABEL_CHARS: usize = 160;
const MAX_LIVE_CATALOGS: usize = 32;
const CATALOG_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RecordingAudioDeviceKind {
    SystemAudio,
    Microphone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeRecordingAudioDevice {
    pub kind: RecordingAudioDeviceKind,
    pub native_id: String,
    pub label: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecordingAudioDeviceSummary {
    id: String,
    label: String,
    is_default: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecordingAudioDeviceCatalogView {
    pub catalog_id: String,
    pub system_audio_devices: Vec<RecordingAudioDeviceSummary>,
    pub microphone_devices: Vec<RecordingAudioDeviceSummary>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordingAudioSelection {
    pub mode: RecordingAudioMode,
    pub catalog_id: Option<String>,
    pub system_device_id: Option<String>,
    pub microphone_device_id: Option<String>,
}

impl RecordingAudioSelection {
    pub(super) const fn default_for(mode: RecordingAudioMode) -> Self {
        Self {
            mode,
            catalog_id: None,
            system_device_id: None,
            microphone_device_id: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ResolvedRecordingAudioDevices {
    pub system_native_id: Option<String>,
    pub microphone_native_id: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum RecordingAudioDeviceCatalogError {
    #[error("录屏音频设备选择包含当前模式不使用的设备")]
    UnexpectedDevice,
    #[error("录屏音频设备 token 缺少目录身份")]
    MissingCatalog,
    #[error("录屏音频设备目录不存在、已刷新或已过期")]
    StaleCatalog,
    #[error("录屏音频设备 token 不存在或类别不匹配")]
    InvalidDevice,
    #[error("录屏音频设备目录锁已损坏")]
    Poisoned,
}

#[derive(Debug, Clone)]
struct CatalogEntry {
    kind: RecordingAudioDeviceKind,
    native_id: String,
}

#[derive(Debug)]
struct Catalog {
    id: String,
    created_at: Instant,
    entries: HashMap<String, CatalogEntry>,
}

#[derive(Default)]
pub(crate) struct RecordingAudioDeviceCatalog {
    next_generation: AtomicU64,
    by_caller: Mutex<HashMap<String, Catalog>>,
}

impl RecordingAudioDeviceCatalog {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(super) fn refresh(
        &self,
        caller: &str,
        devices: Vec<NativeRecordingAudioDevice>,
    ) -> Result<RecordingAudioDeviceCatalogView, RecordingAudioDeviceCatalogError> {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let catalog_id = format!("audio-catalog-{generation:016x}");
        let mut normalized = normalize_devices(devices);
        normalized.sort_by(|left, right| {
            left.kind
                .sort_key()
                .cmp(&right.kind.sort_key())
                .then_with(|| right.is_default.cmp(&left.is_default))
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.native_id.cmp(&right.native_id))
        });

        let mut entries = HashMap::new();
        let mut system_audio_devices = Vec::new();
        let mut microphone_devices = Vec::new();
        for (index, device) in normalized.into_iter().enumerate() {
            let token = format!("audio-device-{generation:016x}-{index:02x}");
            let summary = RecordingAudioDeviceSummary {
                id: token.clone(),
                label: device.label,
                is_default: device.is_default,
            };
            match device.kind {
                RecordingAudioDeviceKind::SystemAudio => system_audio_devices.push(summary),
                RecordingAudioDeviceKind::Microphone => microphone_devices.push(summary),
            }
            entries.insert(
                token,
                CatalogEntry {
                    kind: device.kind,
                    native_id: device.native_id,
                },
            );
        }

        let catalog = Catalog {
            id: catalog_id.clone(),
            created_at: Instant::now(),
            entries,
        };
        let mut catalogs = self
            .by_caller
            .lock()
            .map_err(|_| RecordingAudioDeviceCatalogError::Poisoned)?;
        catalogs.retain(|_, existing| existing.created_at.elapsed() <= CATALOG_TTL);
        catalogs.remove(caller);
        if catalogs.len() >= MAX_LIVE_CATALOGS {
            let oldest = catalogs
                .iter()
                .min_by_key(|(_, existing)| existing.created_at)
                .map(|(caller, _)| caller.clone());
            if let Some(oldest) = oldest {
                catalogs.remove(&oldest);
            }
        }
        catalogs.insert(caller.to_string(), catalog);
        Ok(RecordingAudioDeviceCatalogView {
            catalog_id,
            system_audio_devices,
            microphone_devices,
        })
    }

    pub(super) fn resolve(
        &self,
        caller: &str,
        selection: &RecordingAudioSelection,
    ) -> Result<ResolvedRecordingAudioDevices, RecordingAudioDeviceCatalogError> {
        validate_selection_shape(selection)?;
        let has_explicit_device =
            selection.system_device_id.is_some() || selection.microphone_device_id.is_some();
        if !has_explicit_device {
            if selection.catalog_id.is_some() {
                self.consume_matching_catalog(caller, selection.catalog_id.as_deref())?;
            }
            return Ok(ResolvedRecordingAudioDevices::default());
        }
        let catalog_id = selection
            .catalog_id
            .as_deref()
            .ok_or(RecordingAudioDeviceCatalogError::MissingCatalog)?;
        let catalog = self.consume_matching_catalog(caller, Some(catalog_id))?;
        Ok(ResolvedRecordingAudioDevices {
            system_native_id: resolve_token(
                &catalog,
                selection.system_device_id.as_deref(),
                RecordingAudioDeviceKind::SystemAudio,
            )?,
            microphone_native_id: resolve_token(
                &catalog,
                selection.microphone_device_id.as_deref(),
                RecordingAudioDeviceKind::Microphone,
            )?,
        })
    }

    fn consume_matching_catalog(
        &self,
        caller: &str,
        expected_id: Option<&str>,
    ) -> Result<Catalog, RecordingAudioDeviceCatalogError> {
        let mut catalogs = self
            .by_caller
            .lock()
            .map_err(|_| RecordingAudioDeviceCatalogError::Poisoned)?;
        let catalog = catalogs
            .get(caller)
            .ok_or(RecordingAudioDeviceCatalogError::StaleCatalog)?;
        if expected_id.is_some_and(|expected| expected != catalog.id) {
            return Err(RecordingAudioDeviceCatalogError::StaleCatalog);
        }
        if catalog.created_at.elapsed() > CATALOG_TTL {
            catalogs.remove(caller);
            return Err(RecordingAudioDeviceCatalogError::StaleCatalog);
        }
        catalogs
            .remove(caller)
            .ok_or(RecordingAudioDeviceCatalogError::StaleCatalog)
    }
}

impl RecordingAudioDeviceKind {
    const fn sort_key(self) -> u8 {
        match self {
            Self::SystemAudio => 0,
            Self::Microphone => 1,
        }
    }
}

fn normalize_devices(devices: Vec<NativeRecordingAudioDevice>) -> Vec<NativeRecordingAudioDevice> {
    let mut seen = HashSet::new();
    let mut counts = [0_usize; 2];
    devices
        .into_iter()
        .filter_map(|mut device| {
            // 原生 ID 是平台 API 的不透明身份；只能判空，不能 trim 或改写后再交还平台。
            if device.native_id.trim().is_empty()
                || !seen.insert((device.kind, device.native_id.clone()))
            {
                return None;
            }
            let count = &mut counts[usize::from(device.kind.sort_key())];
            if *count >= MAX_DEVICES_PER_KIND {
                return None;
            }
            *count += 1;
            device.label = normalize_label(&device.label);
            Some(device)
        })
        .collect()
}

fn normalize_label(label: &str) -> String {
    let mut normalized = String::new();
    let mut pending_space = false;
    let mut character_count = 0_usize;
    for character in label.chars() {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if character.is_control() {
            continue;
        }
        // 只在后面还放得下一个实际字符时插入压缩后的空格，避免截断后留下尾随空白。
        if pending_space && character_count + 1 < MAX_LABEL_CHARS {
            normalized.push(' ');
            character_count += 1;
        } else if pending_space {
            break;
        }
        pending_space = false;
        if character_count >= MAX_LABEL_CHARS {
            break;
        }
        normalized.push(character);
        character_count += 1;
    }
    if normalized.is_empty() {
        "Unnamed audio device".to_string()
    } else {
        normalized
    }
}

fn validate_selection_shape(
    selection: &RecordingAudioSelection,
) -> Result<(), RecordingAudioDeviceCatalogError> {
    let valid = match selection.mode {
        RecordingAudioMode::None => {
            selection.system_device_id.is_none() && selection.microphone_device_id.is_none()
        }
        RecordingAudioMode::SystemAudio => selection.microphone_device_id.is_none(),
        RecordingAudioMode::Microphone => selection.system_device_id.is_none(),
        RecordingAudioMode::SystemAndMicrophone => true,
    };
    valid
        .then_some(())
        .ok_or(RecordingAudioDeviceCatalogError::UnexpectedDevice)
}

fn resolve_token(
    catalog: &Catalog,
    token: Option<&str>,
    expected_kind: RecordingAudioDeviceKind,
) -> Result<Option<String>, RecordingAudioDeviceCatalogError> {
    let Some(token) = token else {
        return Ok(None);
    };
    let entry = catalog
        .entries
        .get(token)
        .filter(|entry| entry.kind == expected_kind)
        .ok_or(RecordingAudioDeviceCatalogError::InvalidDevice)?;
    Ok(Some(entry.native_id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(
        kind: RecordingAudioDeviceKind,
        id: impl Into<String>,
        label: impl Into<String>,
        is_default: bool,
    ) -> NativeRecordingAudioDevice {
        NativeRecordingAudioDevice {
            kind,
            native_id: id.into(),
            label: label.into(),
            is_default,
        }
    }

    #[test]
    fn catalog_hides_native_ids_and_resolves_each_kind_once() {
        let registry = RecordingAudioDeviceCatalog::new();
        let view = registry
            .refresh(
                "recording-overlay-a",
                vec![
                    device(
                        RecordingAudioDeviceKind::SystemAudio,
                        "sink-secret",
                        "Speakers",
                        true,
                    ),
                    device(
                        RecordingAudioDeviceKind::Microphone,
                        "mic-secret",
                        "Microphone",
                        false,
                    ),
                ],
            )
            .unwrap();
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("sink-secret"));
        assert!(!json.contains("mic-secret"));
        let selection = RecordingAudioSelection {
            mode: RecordingAudioMode::SystemAndMicrophone,
            catalog_id: Some(view.catalog_id.clone()),
            system_device_id: Some(view.system_audio_devices[0].id.clone()),
            microphone_device_id: Some(view.microphone_devices[0].id.clone()),
        };
        assert_eq!(
            registry.resolve("recording-overlay-a", &selection).unwrap(),
            ResolvedRecordingAudioDevices {
                system_native_id: Some("sink-secret".to_string()),
                microphone_native_id: Some("mic-secret".to_string()),
            }
        );
        assert_eq!(
            registry.resolve("recording-overlay-a", &selection),
            Err(RecordingAudioDeviceCatalogError::StaleCatalog)
        );
    }

    #[test]
    fn refresh_invalidates_old_catalog_without_consuming_current_catalog() {
        let registry = RecordingAudioDeviceCatalog::new();
        let old = registry
            .refresh(
                "recording-overlay-a",
                vec![device(
                    RecordingAudioDeviceKind::Microphone,
                    "old",
                    "Old",
                    false,
                )],
            )
            .unwrap();
        let current = registry
            .refresh(
                "recording-overlay-a",
                vec![device(
                    RecordingAudioDeviceKind::Microphone,
                    "new",
                    "New",
                    true,
                )],
            )
            .unwrap();
        let old_selection = RecordingAudioSelection {
            mode: RecordingAudioMode::Microphone,
            catalog_id: Some(old.catalog_id),
            system_device_id: None,
            microphone_device_id: Some(old.microphone_devices[0].id.clone()),
        };
        assert_eq!(
            registry.resolve("recording-overlay-a", &old_selection),
            Err(RecordingAudioDeviceCatalogError::StaleCatalog)
        );

        let current_selection = RecordingAudioSelection {
            mode: RecordingAudioMode::Microphone,
            catalog_id: Some(current.catalog_id),
            system_device_id: None,
            microphone_device_id: Some(current.microphone_devices[0].id.clone()),
        };
        assert_eq!(
            registry.resolve("recording-overlay-a", &current_selection),
            Ok(ResolvedRecordingAudioDevices {
                system_native_id: None,
                microphone_native_id: Some("new".to_string()),
            })
        );
    }

    #[test]
    fn caller_binding_rejects_foreign_tokens_without_consuming_owner_catalog() {
        let registry = RecordingAudioDeviceCatalog::new();
        let view = registry
            .refresh(
                "recording-overlay-a",
                vec![device(
                    RecordingAudioDeviceKind::Microphone,
                    "mic-secret",
                    "Microphone",
                    true,
                )],
            )
            .unwrap();
        let selection = RecordingAudioSelection {
            mode: RecordingAudioMode::Microphone,
            catalog_id: Some(view.catalog_id),
            system_device_id: None,
            microphone_device_id: Some(view.microphone_devices[0].id.clone()),
        };
        assert_eq!(
            registry.resolve("recording-overlay-b", &selection),
            Err(RecordingAudioDeviceCatalogError::StaleCatalog)
        );
        assert_eq!(
            registry.resolve("recording-overlay-a", &selection),
            Ok(ResolvedRecordingAudioDevices {
                system_native_id: None,
                microphone_native_id: Some("mic-secret".to_string()),
            })
        );
    }

    #[test]
    fn wrong_kind_and_unused_tokens_are_rejected() {
        let registry = RecordingAudioDeviceCatalog::new();
        let view = registry
            .refresh(
                "recording-overlay-a",
                vec![device(
                    RecordingAudioDeviceKind::SystemAudio,
                    "sink",
                    "Sink",
                    true,
                )],
            )
            .unwrap();
        let wrong_kind = RecordingAudioSelection {
            mode: RecordingAudioMode::Microphone,
            catalog_id: Some(view.catalog_id),
            system_device_id: None,
            microphone_device_id: Some(view.system_audio_devices[0].id.clone()),
        };
        assert_eq!(
            registry.resolve("recording-overlay-a", &wrong_kind),
            Err(RecordingAudioDeviceCatalogError::InvalidDevice)
        );

        let unused = RecordingAudioSelection {
            mode: RecordingAudioMode::None,
            catalog_id: None,
            system_device_id: Some("audio-device-forged".to_string()),
            microphone_device_id: None,
        };
        assert_eq!(
            registry.resolve("recording-overlay-a", &unused),
            Err(RecordingAudioDeviceCatalogError::UnexpectedDevice)
        );
    }

    #[test]
    fn normalization_deduplicates_bounds_and_sanitizes_labels() {
        let registry = RecordingAudioDeviceCatalog::new();
        let mut devices = (0..70)
            .map(|index| {
                device(
                    RecordingAudioDeviceKind::Microphone,
                    format!("mic-{index}"),
                    format!("  Mic\n{index}\u{0000}  "),
                    index == 4,
                )
            })
            .collect::<Vec<_>>();
        devices.push(device(
            RecordingAudioDeviceKind::Microphone,
            "mic-4",
            "duplicate",
            false,
        ));
        let view = registry.refresh("recording-overlay-a", devices).unwrap();
        assert_eq!(view.microphone_devices.len(), MAX_DEVICES_PER_KIND);
        assert_eq!(view.microphone_devices[0].label, "Mic 4");
        assert!(view
            .microphone_devices
            .iter()
            .all(|device| !device.label.chars().any(char::is_control)));
    }

    #[test]
    fn normalization_preserves_opaque_native_ids_and_never_leaves_trailing_space() {
        let normalized = normalize_devices(vec![
            device(
                RecordingAudioDeviceKind::SystemAudio,
                "  exact endpoint id  ",
                format!("{} next", "a".repeat(MAX_LABEL_CHARS - 1)),
                false,
            ),
            device(
                RecordingAudioDeviceKind::Microphone,
                "   ",
                "ignored",
                false,
            ),
        ]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].native_id, "  exact endpoint id  ");
        assert_eq!(normalized[0].label, "a".repeat(MAX_LABEL_CHARS - 1));
        assert!(!normalized[0].label.ends_with(char::is_whitespace));
    }

    #[test]
    fn default_selection_does_not_require_a_catalog() {
        let registry = RecordingAudioDeviceCatalog::new();
        assert_eq!(
            registry
                .resolve(
                    "recording-overlay-a",
                    &RecordingAudioSelection::default_for(RecordingAudioMode::SystemAudio),
                )
                .unwrap(),
            ResolvedRecordingAudioDevices::default()
        );
    }

    #[test]
    fn abandoned_catalogs_are_globally_bounded() {
        let registry = RecordingAudioDeviceCatalog::new();
        let mut first = None;
        let mut latest = None;
        for index in 0..(MAX_LIVE_CATALOGS + 4) {
            let view = registry
                .refresh(&format!("recording-overlay-{index}"), Vec::new())
                .unwrap();
            first.get_or_insert_with(|| view.catalog_id.clone());
            latest = Some(view);
        }
        assert_eq!(registry.by_caller.lock().unwrap().len(), MAX_LIVE_CATALOGS);
        assert_eq!(
            registry.resolve(
                "recording-overlay-0",
                &RecordingAudioSelection {
                    mode: RecordingAudioMode::None,
                    catalog_id: first,
                    system_device_id: None,
                    microphone_device_id: None,
                },
            ),
            Err(RecordingAudioDeviceCatalogError::StaleCatalog)
        );
        let latest = latest.unwrap();
        assert_eq!(
            registry.resolve(
                &format!("recording-overlay-{}", MAX_LIVE_CATALOGS + 3),
                &RecordingAudioSelection {
                    mode: RecordingAudioMode::None,
                    catalog_id: Some(latest.catalog_id),
                    system_device_id: None,
                    microphone_device_id: None,
                },
            ),
            Ok(ResolvedRecordingAudioDevices::default())
        );
    }
}
