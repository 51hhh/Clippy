import { AudioLines, Mic, Settings2, Video, Volume2, VolumeX, X } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { t } from "../shared/i18n";
import { toolbarPlacement } from "./geometry";
import type { RecordingAudioDeviceSummary, RecordingAudioMode, Rect } from "./types";

const FALLBACK_SIZE = { width: 76, height: 40 };

type Props = {
  selection: Rect;
  viewportWidth: number;
  viewportHeight: number;
  busy: boolean;
  audioModes: RecordingAudioMode[];
  audioMode: RecordingAudioMode;
  systemAudioDevices: RecordingAudioDeviceSummary[];
  microphoneDevices: RecordingAudioDeviceSummary[];
  systemDeviceId: string | null;
  microphoneDeviceId: string | null;
  onAudioModeChange: (mode: RecordingAudioMode) => void;
  onSystemDeviceChange: (id: string | null) => void;
  onMicrophoneDeviceChange: (id: string | null) => void;
  onStart: () => void;
  onCancel: () => void;
};

/** 独立录屏入口只保留开始与取消；截图标注、扫码和输出动作不进入这个调用域。 */
export function RecordingSelectionToolbar(props: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(FALLBACK_SIZE);
  const [devicesOpen, setDevicesOpen] = useState(false);

  useLayoutEffect(() => {
    const rect = panel.current?.getBoundingClientRect();
    if (!rect) return;
    const next = {
      width: rect.width || FALLBACK_SIZE.width,
      height: rect.height || FALLBACK_SIZE.height,
    };
    setSize((current) =>
      current.width === next.width && current.height === next.height ? current : next,
    );
  });

  const placement = toolbarPlacement(props.selection, size, {
    width: props.viewportWidth,
    height: props.viewportHeight,
  });
  const audioLabel = t(`capture.recording.audio.${props.audioMode}`);

  function cycleAudioMode() {
    if (props.audioModes.length < 2) return;
    const current = props.audioModes.indexOf(props.audioMode);
    props.onAudioModeChange(props.audioModes[(current + 1) % props.audioModes.length]);
  }

  const AudioIcon = props.audioMode === "systemAudio"
    ? Volume2
    : props.audioMode === "microphone"
      ? Mic
      : props.audioMode === "systemAndMicrophone"
        ? AudioLines
        : VolumeX;
  const usesSystemAudio = props.audioMode === "systemAudio"
    || props.audioMode === "systemAndMicrophone";
  const usesMicrophone = props.audioMode === "microphone"
    || props.audioMode === "systemAndMicrophone";
  const hasRelevantDevices = (usesSystemAudio && props.systemAudioDevices.length > 0)
    || (usesMicrophone && props.microphoneDevices.length > 0);

  function deviceOptionLabel(device: RecordingAudioDeviceSummary) {
    return device.isDefault
      ? `${device.label} · ${t("capture.recording.device.currentDefault")}`
      : device.label;
  }

  return (
    <div
      ref={panel}
      className="overlay-toolbar recording-selection-toolbar"
      role="toolbar"
      aria-label={t("capture.recording.toolbar")}
      style={{ left: placement.left, top: placement.top }}
      onPointerDown={(event) => event.stopPropagation()}
      onPointerMove={(event) => event.stopPropagation()}
      onPointerUp={(event) => event.stopPropagation()}
      onPointerCancel={(event) => event.stopPropagation()}
    >
      <div className="overlay-toolbar-row">
        {props.audioModes.length > 1 && (
          <button
            type="button"
            className="recording-audio-mode"
            title={`${t("capture.recording.audio.label")}: ${audioLabel}`}
            aria-label={`${t("capture.recording.audio.label")}: ${audioLabel}`}
            disabled={props.busy}
            onClick={cycleAudioMode}
          >
            <AudioIcon size={16} />
          </button>
        )}
        {hasRelevantDevices && (
          <button
            type="button"
            className={devicesOpen ? "recording-audio-devices active" : "recording-audio-devices"}
            title={t("capture.recording.device.configure")}
            aria-label={t("capture.recording.device.configure")}
            aria-expanded={devicesOpen}
            disabled={props.busy}
            onClick={() => setDevicesOpen((open) => !open)}
          >
            <Settings2 size={16} />
          </button>
        )}
        <button
          type="button"
          className="overlay-confirm"
          title={t("capture.recording.start")}
          aria-label={t("capture.recording.start")}
          disabled={props.busy}
          onClick={props.onStart}
        >
          <Video size={16} />
        </button>
        <button
          type="button"
          title={t("capture.cancel")}
          aria-label={t("capture.cancel")}
          disabled={props.busy}
          onClick={props.onCancel}
        >
          <X size={15} />
        </button>
      </div>
      {devicesOpen && hasRelevantDevices && (
        <div className="recording-device-panel" role="group" aria-label={t("capture.recording.device.configure")}>
          {usesSystemAudio && props.systemAudioDevices.length > 0 && (
            <label>
              <span>{t("capture.recording.device.systemAudio")}</span>
              <select
                aria-label={t("capture.recording.device.systemAudio")}
                disabled={props.busy}
                value={props.systemDeviceId ?? ""}
                onChange={(event) => props.onSystemDeviceChange(event.target.value || null)}
              >
                <option value="">{t("capture.recording.device.followDefault")}</option>
                {props.systemAudioDevices.map((device) => (
                  <option key={device.id} value={device.id}>{deviceOptionLabel(device)}</option>
                ))}
              </select>
            </label>
          )}
          {usesMicrophone && props.microphoneDevices.length > 0 && (
            <label>
              <span>{t("capture.recording.device.microphone")}</span>
              <select
                aria-label={t("capture.recording.device.microphone")}
                disabled={props.busy}
                value={props.microphoneDeviceId ?? ""}
                onChange={(event) => props.onMicrophoneDeviceChange(event.target.value || null)}
              >
                <option value="">{t("capture.recording.device.followDefault")}</option>
                {props.microphoneDevices.map((device) => (
                  <option key={device.id} value={device.id}>{deviceOptionLabel(device)}</option>
                ))}
              </select>
            </label>
          )}
        </div>
      )}
    </div>
  );
}
