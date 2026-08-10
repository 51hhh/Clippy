import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useState } from "react";
import { overlayApi } from "./api";
import { OverlayToolbar } from "./OverlayToolbar";
import type { CaptureAction, CaptureOverlayPayload, Point, ResizeHandle } from "./types";
import { useSelection } from "./useSelection";

const HANDLES: ResizeHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

function objectUrl(base64: string): string {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

export function App() {
  const label = getCurrentWindow().label;
  const [payload, setPayload] = useState<CaptureOverlayPayload | null>(null);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selection = useSelection(payload?.logicalWidth || 1, payload?.logicalHeight || 1, payload?.windows || []);

  useEffect(() => {
    let cancelled = false;
    overlayApi.get(label).then((value) => !cancelled && setPayload(value)).catch((reason) => !cancelled && setError(String(reason)));
    return () => { cancelled = true; };
  }, [label]);

  useEffect(() => {
    if (!payload) return;
    const url = objectUrl(payload.pngBase64);
    setImageUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [payload]);

  const cancel = useCallback(() => {
    if (payload && !busy) {
      setBusy(true);
      overlayApi.cancel(payload.sessionId).catch((reason) => {
        setError(String(reason));
        setBusy(false);
      });
    }
  }, [busy, payload]);

  const run = useCallback((action: CaptureAction) => {
    if (!payload || !selection.selection || busy) return;
    setBusy(true);
    setError(null);
    overlayApi.run(action, {
      ...selection.selection,
      sessionId: payload.sessionId,
      monitorId: payload.monitorId,
    }).catch((reason) => {
      setError(String(reason));
      setBusy(false);
    });
  }, [busy, payload, selection.selection]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") cancel();
      else if (event.key === "Enter" && selection.selection) run("copy");
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancel, run, selection.selection]);

  const imageStyle = useMemo(() => ({ width: payload?.logicalWidth || 1, height: payload?.logicalHeight || 1 }), [payload]);
  function point(event: React.PointerEvent): Point {
    return { x: event.clientX, y: event.clientY };
  }

  if (!payload || !imageUrl) return <main className="overlay-root loading" />;
  const selected = selection.selection;
  const preview = !selected ? selection.candidate : null;

  return (
    <main
      className="overlay-root"
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        selection.pointerDown(point(event));
      }}
      onPointerMove={(event) => selection.pointerMove(point(event))}
      onPointerUp={(event) => selection.pointerUp(point(event))}
      onPointerCancel={(event) => selection.pointerUp(point(event))}
    >
      <img className="overlay-frame" src={imageUrl} alt="" draggable={false} style={imageStyle} />
      <div className="overlay-shade" />
      {preview && (
        <div
          className="window-preview"
          style={{ left: preview.x, top: preview.y, width: preview.width, height: preview.height }}
        />
      )}
      {selected && (
        <>
          <div
            className="selection"
            style={{
              left: selected.x,
              top: selected.y,
              width: selected.width,
              height: selected.height,
              backgroundImage: `url(${imageUrl})`,
              backgroundPosition: `-${selected.x}px -${selected.y}px`,
              backgroundSize: `${payload.logicalWidth}px ${payload.logicalHeight}px`,
            }}
          >
            {HANDLES.map((handle) => <span key={handle} className={`selection-handle ${handle}`} />)}
          </div>
          <div className="selection-size" style={{ left: selected.x, top: Math.max(6, selected.y - 28) }}>
            {Math.round(selected.width)} × {Math.round(selected.height)}
          </div>
          {selected.width >= 2 && selected.height >= 2 && (
            <OverlayToolbar
              selection={selected}
              viewportWidth={payload.logicalWidth}
              viewportHeight={payload.logicalHeight}
              busy={busy}
              onAction={run}
              onCancel={cancel}
            />
          )}
        </>
      )}
      {error && <div className="overlay-error" role="status">{error}</div>}
    </main>
  );
}
