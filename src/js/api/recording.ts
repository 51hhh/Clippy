import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  CaptureSelection,
  RecordingLibraryItem,
  RecordingLibrarySettings,
  RecordingPlaybackLease,
  RecordingStopResult,
} from "../ipc-types.ts";

/** 从后端签发的 Recording 覆盖层提交逻辑选区；物理 crop 与编码策略仍由后端决定。 */
export function startCaptureRecording(selection: CaptureSelection): Promise<void> {
  return invoke<void>("start_capture_recording", { selection });
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
