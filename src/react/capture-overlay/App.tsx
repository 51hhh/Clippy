import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindowLabel } from "../../js/api.ts";
import { overlayApi } from "./api";
import { OverlayToolbar } from "./OverlayToolbar";
import { TranslationPopover } from "./TranslationPopover";
import {
  isCurrentTranslation,
  translationErrorMessage,
  translationPanelPosition,
} from "./translationState";
import type {
  CaptureAction,
  CaptureOverlayPayload,
  CaptureTranslationState,
  Point,
  ResizeHandle,
} from "./types";
import { useSelection } from "./useSelection";

const HANDLES: ResizeHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

function objectUrl(base64: string): string {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

export function App() {
  const label = getCurrentWindowLabel();
  const [payload, setPayload] = useState<CaptureOverlayPayload | null>(null);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [translation, setTranslation] = useState<CaptureTranslationState | null>(null);
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);
  const translationGeneration = useRef(0);
  const translateButtonRef = useRef<HTMLButtonElement>(null);
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
      translationGeneration.current += 1;
      setTranslation(null);
      setCopyStatus(null);
      setBusy(true);
      overlayApi.cancel(payload.sessionId).catch((reason) => {
        setError(String(reason));
        setBusy(false);
      });
    }
  }, [busy, payload]);

  const run = useCallback((action: CaptureAction) => {
    if (!payload || !selection.selection || busy || translation?.status === "loading") return;
    translationGeneration.current += 1;
    setTranslation(null);
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
  }, [busy, payload, selection.selection, translation?.status]);

  const closeTranslation = useCallback(() => {
    translationGeneration.current += 1;
    setTranslation(null);
    setCopyStatus(null);
    requestAnimationFrame(() => translateButtonRef.current?.focus());
  }, []);

  const translate = useCallback(() => {
    if (!payload || !selection.selection || busy || translation?.status === "loading") return;
    const generation = translationGeneration.current + 1;
    translationGeneration.current = generation;
    setCopyStatus(null);
    setTranslation({ status: "loading" });
    overlayApi.translate({
      ...selection.selection,
      sessionId: payload.sessionId,
      monitorId: payload.monitorId,
    }).then((result) => {
      if (isCurrentTranslation(translationGeneration.current, generation)) {
        setTranslation({ status: "result", result });
      }
    }).catch((reason) => {
      if (isCurrentTranslation(translationGeneration.current, generation)) {
        setTranslation({ status: "error", message: translationErrorMessage(reason) });
      }
    });
  }, [busy, payload, selection.selection, translation?.status]);

  const copyTranslation = useCallback(async () => {
    if (translation?.status !== "result") return;
    try {
      await overlayApi.copyText(translation.result.translatedText);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("failed");
    }
  }, [translation]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && translation) {
        event.preventDefault();
        closeTranslation();
      } else if (event.key === "Escape") {
        cancel();
      } else if (
        event.key === "Enter"
        && selection.selection
        && !(event.target instanceof HTMLElement && event.target.closest("button"))
      ) {
        run("copy");
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancel, closeTranslation, run, selection.selection, translation]);

  const imageStyle = useMemo(() => ({ width: payload?.logicalWidth || 1, height: payload?.logicalHeight || 1 }), [payload]);
  function point(event: React.PointerEvent): Point {
    return { x: event.clientX, y: event.clientY };
  }

  if (!payload || !imageUrl) return <main className="overlay-root loading" />;
  const selected = selection.selection;
  const preview = !selected ? selection.candidate : null;
  const translationPosition = selected && translation
    ? translationPanelPosition(selected, payload.logicalWidth, payload.logicalHeight)
    : null;

  return (
    <main
      className="overlay-root"
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        if (translation) closeTranslation();
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
              translationBusy={translation?.status === "loading"}
              onAction={run}
              onTranslate={translate}
              onCancel={cancel}
              translateButtonRef={translateButtonRef}
            />
          )}
        </>
      )}
      {translation && selected && translationPosition && (
        <TranslationPopover
          state={translation}
          left={translationPosition.left}
          top={translationPosition.top}
          copyStatus={copyStatus}
          onCopy={() => void copyTranslation()}
          onClose={closeTranslation}
        />
      )}
      {error && <div className="overlay-error" role="status">{error}</div>}
    </main>
  );
}
