import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindowLabel } from "../../js/api.ts";
import { t } from "../shared/i18n";
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
  Rect,
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

  useEffect(() => {
    // 排障线索：窗口几何拿不到时界面只有一句提示，日志里要留下原因可查。
    if (payload && payload.windows.length === 0) {
      console.warn("截图覆盖层未拿到窗口几何，窗口速选不可用");
    }
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

  // rect 显式传入是为了刚松手那一刻就能动作：此时 selection 状态还没提交。
  const run = useCallback((action: CaptureAction, rect: Rect | null = selection.selection) => {
    if (!payload || !rect || busy || translation?.status === "loading") return;
    translationGeneration.current += 1;
    setTranslation(null);
    setBusy(true);
    setError(null);
    overlayApi.run(action, {
      ...rect,
      sessionId: payload.sessionId,
      monitorId: payload.monitorId,
    }).catch((reason) => {
      setError(String(reason));
      setBusy(false);
    });
  }, [busy, payload, selection.selection, translation?.status]);

  /**
   * 框选落地：配置成 "editor" 时直接开编辑器（参考项目的手感），否则停在工具条上。
   * 松手时按住 Alt 可以临时留在工具条上，否则选区翻译在默认配置下就没有入口了。
   */
  const commit = useCallback((rect: Rect | null, keepToolbar: boolean) => {
    if (rect && !keepToolbar && payload?.commitAction === "editor") run("edit", rect);
  }, [payload?.commitAction, run]);

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
  // 选区外面继续给窗口预览，速选不会因为框过一次就消失。
  const preview = selection.candidate;
  // 拿不到窗口列表（部分 Wayland 合成器不给窗口几何）时明说，否则用户只会觉得速选坏了。
  const windowPickingUnavailable = payload.windows.length === 0;
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
      onPointerUp={(event) => commit(selection.pointerUp(point(event)), event.altKey)}
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
      {!selected && !error && windowPickingUnavailable && (
        <div className="overlay-hint" role="status">{t("capture.windowPickingUnavailable")}</div>
      )}
      {error && <div className="overlay-error" role="status">{error}</div>}
    </main>
  );
}
