import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  CaptureSelection,
  RecordingLibraryItem,
  RecordingLibrarySettings,
  RecordingPlaybackLease,
  RecordingAudioMode,
  RecordingAudioSelection,
  RecordingStartCapabilities,
  RecordingStopResult,
} from "../ipc-types.ts";

const RECORDING_AUDIO_MODES = new Set<RecordingAudioMode>([
  "none",
  "systemAudio",
  "microphone",
  "systemAndMicrophone",
]);
const AUDIO_CATALOG_ID = /^audio-catalog-[a-f0-9]{16}$/;
const AUDIO_DEVICE_ID = /^audio-device-[a-f0-9]{16}-[a-f0-9]{2}$/;

function validDeviceSummary(value: unknown): value is {
  id: string;
  label: string;
  isDefault: boolean;
} {
  if (!value || typeof value !== "object") return false;
  const device = value as { id?: unknown; label?: unknown; isDefault?: unknown };
  return typeof device.id === "string"
    && AUDIO_DEVICE_ID.test(device.id)
    && typeof device.label === "string"
    && device.label.length > 0
    && Array.from(device.label).length <= 160
    && !/[\u0000-\u001f\u007f]/u.test(device.label)
    && typeof device.isDefault === "boolean";
}

export async function getRecordingStartCapabilities(): Promise<RecordingStartCapabilities> {
  const capabilities = await invoke<RecordingStartCapabilities>("get_recording_start_capabilities");
  const audioModes = capabilities?.audioModes;
  const catalog = capabilities?.deviceCatalog;
  const systemAudioDevices = catalog?.systemAudioDevices;
  const microphoneDevices = catalog?.microphoneDevices;
  const deviceIds = [
    ...(Array.isArray(systemAudioDevices) ? systemAudioDevices : []),
    ...(Array.isArray(microphoneDevices) ? microphoneDevices : []),
  ].map((device) => device?.id);
  if (
    !Array.isArray(audioModes)
    || audioModes.length < 1
    || audioModes.length > RECORDING_AUDIO_MODES.size
    || audioModes[0] !== "none"
    || new Set(audioModes).size !== audioModes.length
    || audioModes.some((mode) => !RECORDING_AUDIO_MODES.has(mode))
    || !catalog
    || typeof catalog.catalogId !== "string"
    || !AUDIO_CATALOG_ID.test(catalog.catalogId)
    || !Array.isArray(systemAudioDevices)
    || systemAudioDevices.length > 64
    || systemAudioDevices.some((device) => !validDeviceSummary(device))
    || !Array.isArray(microphoneDevices)
    || microphoneDevices.length > 64
    || microphoneDevices.some((device) => !validDeviceSummary(device))
    || new Set(deviceIds).size !== deviceIds.length
    || typeof capabilities.deviceEnumerationFailed !== "boolean"
  ) {
    throw new Error("recording.invalid_start_capabilities");
  }
  return {
    audioModes: [...audioModes],
    deviceCatalog: {
      catalogId: catalog.catalogId,
      systemAudioDevices: systemAudioDevices.map((device) => ({ ...device })),
      microphoneDevices: microphoneDevices.map((device) => ({ ...device })),
    },
    deviceEnumerationFailed: capabilities.deviceEnumerationFailed,
  };
}

/** 从后端签发的 Recording 覆盖层提交逻辑选区；物理 crop 与编码策略仍由后端决定。 */
export function startCaptureRecording(
  selection: CaptureSelection,
  audioSelection: RecordingAudioSelection,
): Promise<void> {
  const mode = audioSelection?.mode;
  const catalogId = audioSelection?.catalogId;
  const systemDeviceId = audioSelection?.systemDeviceId;
  const microphoneDeviceId = audioSelection?.microphoneDeviceId;
  const tokenShapeValid = (value: unknown) => value === null
    || (typeof value === "string" && AUDIO_DEVICE_ID.test(value));
  const modeShapeValid = mode === "none"
    ? systemDeviceId === null && microphoneDeviceId === null
    : mode === "systemAudio"
      ? microphoneDeviceId === null
      : mode === "microphone"
        ? systemDeviceId === null
        : mode === "systemAndMicrophone";
  if (
    !RECORDING_AUDIO_MODES.has(mode)
    || !modeShapeValid
    || !(catalogId === null || (typeof catalogId === "string" && AUDIO_CATALOG_ID.test(catalogId)))
    || !tokenShapeValid(systemDeviceId)
    || !tokenShapeValid(microphoneDeviceId)
    || ((systemDeviceId !== null || microphoneDeviceId !== null) && catalogId === null)
  ) {
    return Promise.reject(new Error("recording.invalid_audio_mode"));
  }
  return invoke<void>("start_capture_recording", { selection, audioSelection });
}

/** 录屏控制页已完成首帧布局；caller label 由 Tauri 注入。 */
export function markRecordingControlReady(): Promise<void> {
  return invoke<void>("mark_recording_control_ready");
}

export function pauseRecording(): Promise<void> {
  return invoke<void>("pause_recording");
}

export function resumeRecording(): Promise<void> {
  return invoke<void>("resume_recording");
}

export function pollRecordingHealth(): Promise<void> {
  return invoke<void>("poll_recording_health");
}

export function stopRecording(): Promise<RecordingStopResult> {
  return invoke<RecordingStopResult>("stop_recording");
}

export function cancelRecording(): Promise<void> {
  return invoke<void>("cancel_recording");
}

export function getRecordingLibrarySettings(): Promise<RecordingLibrarySettings> {
  return invoke<RecordingLibrarySettings>("get_recording_library_settings");
}

export function listRecordings(): Promise<RecordingLibraryItem[]> {
  return invoke<RecordingLibraryItem[]>("list_recordings");
}

export async function getRecordingThumbnail(sessionId: string): Promise<string | null> {
  const thumbnail = await invoke<string | null>("get_recording_thumbnail", { sessionId });
  if (thumbnail === null) return null;
  if (
    typeof thumbnail !== "string"
    || thumbnail.length === 0
    || thumbnail.length > 700_000
    || !/^[A-Za-z0-9+/]+={0,2}$/.test(thumbnail)
  ) {
    throw new Error("recordings.invalid_thumbnail");
  }
  return thumbnail;
}

export function recordingLibraryReady(): Promise<void> {
  return invoke<void>("recording_library_ready");
}

export function startRecordingLibraryDrag(): Promise<void> {
  return invoke<void>("start_recording_library_drag");
}

export function closeRecordingLibrary(): Promise<void> {
  return invoke<void>("close_recording_library");
}

export async function prepareRecordingPlayback(
  sessionId: string,
  artifactId: string,
): Promise<RecordingPlaybackLease> {
  const lease = await invoke<RecordingPlaybackLease>("prepare_recording_playback", {
    sessionId,
    artifactId,
  });
  if (!/^media-[a-f0-9]{16}$/.test(lease?.token) || lease.mimeType !== "video/webm") {
    throw new Error("recordings.invalid_playback_lease");
  }
  return lease;
}

export function releaseRecordingPlayback(token: string): Promise<void> {
  return invoke<void>("release_recording_playback", { token });
}

export function mergeRecordingSession(sessionId: string): Promise<void> {
  return invoke<void>("merge_recording_session", { sessionId });
}

export function getRecordingMediaUrl(token: string): string {
  if (!/^media-[a-f0-9]{16}$/.test(token)) throw new Error("recordings.invalid_playback_lease");
  return convertFileSrc(token, "recording-media");
}

export function exportRecordingArtifact(sessionId: string, artifactId: string): Promise<boolean> {
  return invoke<boolean>("export_recording_artifact", { sessionId, artifactId });
}

export function revealRecordingArtifact(sessionId: string, artifactId: string): Promise<void> {
  return invoke<void>("reveal_recording_artifact", { sessionId, artifactId });
}

export function deleteRecordingSession(sessionId: string): Promise<void> {
  return invoke<void>("delete_recording_session", { sessionId });
}
