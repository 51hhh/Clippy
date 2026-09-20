import { useEffect, useRef, useState } from "react";
import {
  markRecordingControlReady,
  pauseRecording,
  resumeRecording,
  stopRecording,
} from "../../js/api.ts";

function formatElapsed(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours > 0
    ? `${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}:${remainder.toString().padStart(2, "0")}`
    : `${minutes.toString().padStart(2, "0")}:${remainder.toString().padStart(2, "0")}`;
}

export type RecordingControlServices = {
  ready: typeof markRecordingControlReady;
  pause: typeof pauseRecording;
  resume: typeof resumeRecording;
  stop: typeof stopRecording;
};

const defaultServices: RecordingControlServices = {
  ready: markRecordingControlReady,
  pause: pauseRecording,
  resume: resumeRecording,
  stop: stopRecording,
};

export function App({ services = defaultServices }: { services?: RecordingControlServices } = {}) {
  const startedAt = useRef(performance.now());
  const pausedAt = useRef<number | null>(null);
  const pausedTotal = useRef(0);
  const [elapsed, setElapsed] = useState(0);
  const [paused, setPaused] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    const update = () => {
      if (!active || pausedAt.current !== null) return;
      setElapsed(performance.now() - startedAt.current - pausedTotal.current);
    };
    update();
    const timer = window.setInterval(update, 250);
    void services.ready().catch(() => {
      if (active) setFailed(true);
    });
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [services]);

  const togglePause = async () => {
    if (busy || failed) return;
    setBusy(true);
    try {
      if (paused) {
        await services.resume();
        const now = performance.now();
        pausedTotal.current += now - (pausedAt.current ?? now);
        pausedAt.current = null;
        setPaused(false);
      } else {
        await services.pause();
        pausedAt.current = performance.now();
        setPaused(true);
      }
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    if (busy || failed) return;
    setBusy(true);
    try {
      await services.stop();
    } catch {
      setFailed(true);
      setBusy(false);
    }
  };

  return (
    <main className="recording-control" aria-label="Recording controls">
      <span className={`recording-dot${paused ? " paused" : ""}`} aria-hidden="true" />
      <output className="recording-time" aria-label="Elapsed time">
        {formatElapsed(elapsed)}
      </output>
      <button
        type="button"
        className="control-button"
        aria-label={paused ? "Resume recording" : "Pause recording"}
        title={paused ? "Resume" : "Pause"}
        disabled={busy || failed}
        onClick={() => void togglePause()}
      >
        {paused ? "▶" : "Ⅱ"}
      </button>
      <button
        type="button"
        className="control-button stop-button"
        aria-label="Stop recording"
        title="Stop"
        disabled={busy || failed}
        onClick={() => void stop()}
      >
        ■
      </button>
      {failed && <span className="recording-error" role="status">Control failed</span>}
    </main>
  );
}

export { formatElapsed };
