import { Mic, Video, Volume2, VolumeX, X } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { t } from "../shared/i18n";
import { toolbarPlacement } from "./geometry";
import type { RecordingAudioMode, Rect } from "./types";

const FALLBACK_SIZE = { width: 76, height: 40 };

type Props = {
  selection: Rect;
  viewportWidth: number;
  viewportHeight: number;
  busy: boolean;
  audioModes: RecordingAudioMode[];
  audioMode: RecordingAudioMode;
  onAudioModeChange: (mode: RecordingAudioMode) => void;
  onStart: () => void;
  onCancel: () => void;
};

/** 独立录屏入口只保留开始与取消；截图标注、扫码和输出动作不进入这个调用域。 */
export function RecordingSelectionToolbar(props: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(FALLBACK_SIZE);

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
      : VolumeX;

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
    </div>
  );
}
