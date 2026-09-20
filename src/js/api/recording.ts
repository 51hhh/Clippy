import { invoke } from "@tauri-apps/api/core";
import type { CaptureSelection, RecordingStopResult } from "../ipc-types.ts";

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
