import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindowLabel } from "../../js/api.ts";
import { drawScene } from "../annotation/canvasRenderer";
import {
  DEFAULT_IMAGE_ADJUSTMENTS,
  type ImageAdjustments,
} from "../annotation/imageAdjustments";
import { exportPngBase64, pngBase64ToObjectUrl } from "../annotation/pngPipeline";
import type { Annotation, Tool } from "../annotation/types";
import { useCanvasInteractions } from "../annotation/useCanvasInteractions";
import { useHistory } from "../annotation/useHistory";
import { t } from "../shared/i18n";
import { overlayApi } from "./api";
import { toPixelRect } from "./geometry";
import { OverlayToolbar } from "./OverlayToolbar";
import { DEFAULT_COLOR, DEFAULT_STROKE } from "./tools";
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
  OverlayTool,
  Point,
  Rect,
  ResizeHandle,
} from "./types";
import { useSelection } from "./useSelection";

const HANDLES: ResizeHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/**
 * 冻结画面覆盖层：选区、标注和提交全部发生在这一个窗口里。
 *
 * 状态机只有三态，由选区和当前工具推导，不额外存：
 * - idle：没有选区。悬停窗口高亮可速选，点一下取悬停窗口、点空地取整屏。
 * - dragging：按住不放，正在框选 / 移动 / 缩放选区，或正在画一笔标注。
 * - editing：有选区。工具条贴在选区旁边，选区仍可拖动与缩放，
 *   点对钩就把"裁剪 + 标注"后的 PNG 直接送进剪贴板。
 */
export function App() {
  const label = getCurrentWindowLabel();
  const [payload, setPayload] = useState<CaptureOverlayPayload | null>(null);
  const [imageReady, setImageReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tool, setTool] = useState<OverlayTool>("select");
  const [color, setColor] = useState(DEFAULT_COLOR);
  const [stroke, setStroke] = useState(DEFAULT_STROKE);
  const [text, setText] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [adjustments, setAdjustments] = useState<ImageAdjustments>(DEFAULT_IMAGE_ADJUSTMENTS);
  const [translation, setTranslation] = useState<CaptureTranslationState | null>(null);
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);
  const translationGeneration = useRef(0);
  const translateButtonRef = useRef<HTMLButtonElement>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const activeDrag = useRef<"selection" | "canvas" | null>(null);
  const revealed = useRef(false);

  /**
   * 让后端把覆盖层显示出来。窗口是隐藏建窗的：加载 webview、取 payload、解 PNG 的
   * 整段时间里显示出来就是一整屏白色，所以显示时机由这里决定——首帧画好之后，
   * 或者已经有错误可以显示的时候。
   */
  const reveal = useCallback(() => {
    if (revealed.current) return;
    revealed.current = true;
    // 显示失败不该拖住截图：后端还有超时兜底会把窗口显示出来。
    overlayApi.ready(label).catch((reason) => console.warn("覆盖层显示失败", reason));
  }, [label]);

  const logicalWidth = payload?.logicalWidth || 1;
  const logicalHeight = payload?.logicalHeight || 1;
  const region = useSelection(logicalWidth, logicalHeight, payload?.windows || []);
  const history = useHistory<Annotation[]>([]);
  const annotations = history.value;

  // 冻结帧是物理像素，界面是逻辑像素。标注一律存在像素空间里，
  // 这样导出时不必二次换算，缩放显示器上也不会错位。
  const scale = useMemo(
    () => (payload ? payload.logicalWidth / Math.max(1, payload.pixelWidth) : 1),
    [payload],
  );
  const scaleY = useMemo(
    () => (payload ? payload.logicalHeight / Math.max(1, payload.pixelHeight) : 1),
    [payload],
  );

  const canvas = useCanvasInteractions({
    imageRef,
    canvasRef,
    scale,
    // 选区由 useSelection 管，画布侧不需要 crop 工具；万一事件误路由到这里，
    // "crop" 是最无害的落点（它只会写这两个被丢弃的回调）。
    tool: (tool === "select" ? "crop" : tool) as Tool,
    color,
    size: stroke,
    text,
    annotations,
    selection: null,
    setSelection: () => {},
    onSelect: (annotation) => setSelectedId(annotation?.id ?? null),
    commitAnnotations: history.commit,
  });

  useEffect(() => {
    let cancelled = false;
    overlayApi
      .get(label)
      .then((value) => !cancelled && setPayload(value))
      .catch((reason) => !cancelled && setError(String(reason)));
    return () => {
      cancelled = true;
    };
  }, [label]);

  useEffect(() => {
    if (!payload) return;
    const url = pngBase64ToObjectUrl(payload.pngBase64);
    const image = new Image();
    let cancelled = false;
    image.onload = () => {
      if (cancelled) return;
      imageRef.current = image;
      setImageReady(true);
    };
    image.onerror = () => !cancelled && setError(t("capture.decodeFailed"));
    image.src = url;
    return () => {
      cancelled = true;
      imageRef.current = null;
      setImageReady(false);
      URL.revokeObjectURL(url);
    };
  }, [payload]);

  useEffect(() => {
    // 排障线索：窗口几何拿不到时界面只有一句提示，日志里要留下原因可查。
    if (payload && payload.windows.length === 0) {
      console.warn("截图覆盖层未拿到窗口几何，窗口速选不可用");
    }
  }, [payload]);

  const selection = region.selection;
  const frame = useMemo(
    () => ({
      x: 0,
      y: 0,
      width: payload?.pixelWidth || 1,
      height: payload?.pixelHeight || 1,
    }),
    [payload],
  );
  const cropInPixels = useMemo(
    () => (selection ? toPixelRect(selection, 1 / scale, 1 / scaleY, frame) : null),
    [frame, scale, scaleY, selection],
  );

  // `crop` 草稿不是注解（选区由 useSelection 管），画布只关心能画出来的那几种。
  const draftAnnotation = canvas.draft && "annotation" in canvas.draft
    ? canvas.draft.annotation
    : null;

  // 底图、标注、裁剪压暗全部由这一块画布画出来，因此每次状态变化都要重绘。
  useEffect(() => {
    const target = canvasRef.current;
    const image = imageRef.current;
    if (!target || !image || !imageReady) return;
    drawScene(
      target,
      image,
      { width: logicalWidth, height: logicalHeight, fitScale: scale, zoom: 1, scale },
      cropInPixels,
      annotations,
      draftAnnotation,
      adjustments,
      selectedId,
    );
    // 首帧已经落在画布上，可以显示窗口了。
    reveal();
  }, [
    adjustments,
    annotations,
    draftAnnotation,
    cropInPixels,
    imageReady,
    logicalHeight,
    logicalWidth,
    reveal,
    scale,
    selectedId,
  ]);

  // 出错时也要把窗口显示出来，否则用户只看到截图"没反应"，错误提示压根没露面。
  useEffect(() => {
    if (error) reveal();
  }, [error, reveal]);

  const cancel = useCallback(() => {
    if (!payload || busy) return;
    translationGeneration.current += 1;
    setTranslation(null);
    setCopyStatus(null);
    setBusy(true);
    overlayApi.cancel(payload.sessionId).catch((reason) => {
      setError(String(reason));
      setBusy(false);
    });
  }, [busy, payload]);

  /**
   * 提交：把画布渲染的 PNG（裁剪 + 图像调整 + 矢量标注已经合成好）交给后端落地。
   * 成功后后端会关掉覆盖层，所以这里不复位 busy。
   */
  const run = useCallback(
    (action: CaptureAction) => {
      const image = imageRef.current;
      if (!payload || !image || !cropInPixels || busy || translation?.status === "loading") return;
      translationGeneration.current += 1;
      setTranslation(null);
      setBusy(true);
      setError(null);
      exportPngBase64(image, cropInPixels, annotations, adjustments)
        .then((png) => overlayApi.commit(action, payload.sessionId, png))
        .catch((reason) => {
          setError(String(reason));
          setBusy(false);
        });
    },
    [adjustments, annotations, busy, cropInPixels, payload, translation?.status],
  );

  const closeTranslation = useCallback(() => {
    translationGeneration.current += 1;
    setTranslation(null);
    setCopyStatus(null);
    requestAnimationFrame(() => translateButtonRef.current?.focus());
  }, []);

  const translate = useCallback(() => {
    if (!payload || !selection || busy || translation?.status === "loading") return;
    const generation = translationGeneration.current + 1;
    translationGeneration.current = generation;
    setCopyStatus(null);
    setTranslation({ status: "loading" });
    overlayApi
      .translate({ ...selection, sessionId: payload.sessionId, monitorId: payload.monitorId })
      .then((result) => {
        if (isCurrentTranslation(translationGeneration.current, generation)) {
          setTranslation({ status: "result", result });
        }
      })
      .catch((reason) => {
        if (isCurrentTranslation(translationGeneration.current, generation)) {
          setTranslation({ status: "error", message: translationErrorMessage(reason) });
        }
      });
  }, [busy, payload, selection, translation?.status]);

  const copyTranslation = useCallback(async () => {
    if (translation?.status !== "result") return;
    try {
      await overlayApi.copyText(translation.result.translatedText);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("failed");
    }
  }, [translation]);

  const deleteObject = useCallback(() => {
    if (!selectedId) return;
    history.commit((items) => items.filter((item) => item.id !== selectedId));
    setSelectedId(null);
  }, [history, selectedId]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const typing = event.target instanceof HTMLElement && event.target.tagName === "INPUT";
      if (event.key === "Escape") {
        event.preventDefault();
        if (translation) closeTranslation();
        else cancel();
        return;
      }
      if (typing) return;
      const control = event.ctrlKey || event.metaKey;
      if (control && event.key.toLowerCase() === "z") {
        event.preventDefault();
        if (event.shiftKey) history.redo();
        else history.undo();
      } else if (control && event.key.toLowerCase() === "y") {
        event.preventDefault();
        history.redo();
      } else if ((event.key === "Delete" || event.key === "Backspace") && selectedId) {
        event.preventDefault();
        deleteObject();
      } else if (
        event.key === "Enter"
        && selection
        && !(event.target instanceof HTMLElement && event.target.closest("button"))
      ) {
        run("copy");
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancel, closeTranslation, deleteObject, history, run, selectedId, selection, translation]);

  function point(event: React.PointerEvent): Point {
    return { x: event.clientX, y: event.clientY };
  }

  /**
   * 指针路由：缩放手柄永远归选区（这样标注工具激活时依旧能改选区），
   * 其余情况下绘制工具归画布、`select` 工具归选区。
   */
  function onPointerDown(event: React.PointerEvent) {
    if (event.button !== 0) return;
    if (translation) closeTranslation();
    const at = point(event);
    const onHandle = region.handleAt(at) !== null;
    if (tool !== "select" && !onHandle) {
      activeDrag.current = "canvas";
      canvas.onPointerDown(event);
      return;
    }
    activeDrag.current = "selection";
    event.currentTarget.setPointerCapture(event.pointerId);
    region.pointerDown(at);
  }

  function onPointerMove(event: React.PointerEvent) {
    if (activeDrag.current === "canvas") canvas.onPointerMove(event);
    else region.pointerMove(point(event));
  }

  function onPointerUp(event: React.PointerEvent) {
    const owner = activeDrag.current;
    activeDrag.current = null;
    if (owner === "canvas") canvas.onPointerUp();
    else region.pointerUp(point(event));
  }

  if (!payload) {
    return (
      <main className="overlay-root loading">
        {error && <div className="overlay-error" role="status">{error}</div>}
      </main>
    );
  }
  // 选区外面继续给窗口预览，速选不会因为框过一次就消失；
  // 但标注工具激活时不给高亮，否则画笔在选区外扫一下就闪一个蓝框。
  const preview = tool === "select" ? region.candidate : null;
  // 拿不到窗口列表（部分 Wayland 合成器不给窗口几何）时明说，否则用户只会觉得速选坏了。
  const windowPickingUnavailable = payload.windows.length === 0;
  const translationPosition = selection && translation
    ? translationPanelPosition(selection, logicalWidth, logicalHeight)
    : null;

  return (
    <main
      className="overlay-root"
      data-tool={tool}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onContextMenu={(event) => {
        // 右键回到"还没框选"，这样点一次取了整屏之后还能重新框小区域。
        event.preventDefault();
        region.reset();
        setSelectedId(null);
      }}
    >
      <canvas
        ref={canvasRef}
        className="overlay-canvas"
        style={{ width: logicalWidth, height: logicalHeight }}
      />
      {!selection && <div className="overlay-shade" />}
      {preview && (
        <div
          className="window-preview"
          style={{ left: preview.x, top: preview.y, width: preview.width, height: preview.height }}
        />
      )}
      {selection && (
        <>
          <div
            className="selection"
            style={{
              left: selection.x,
              top: selection.y,
              width: selection.width,
              height: selection.height,
            }}
          >
            {HANDLES.map((handle) => <span key={handle} className={`selection-handle ${handle}`} />)}
          </div>
          <div
            className="selection-size"
            style={{ left: selection.x, top: Math.max(6, selection.y - 28) }}
          >
            {Math.round(selection.width)} × {Math.round(selection.height)}
          </div>
          {selection.width >= 2 && selection.height >= 2 && (
            <OverlayToolbar
              selection={selection}
              viewportWidth={logicalWidth}
              viewportHeight={logicalHeight}
              tool={tool}
              color={color}
              stroke={stroke}
              text={text}
              adjustments={adjustments}
              busy={busy}
              translationBusy={translation?.status === "loading"}
              canUndo={history.canUndo}
              canRedo={history.canRedo}
              hasSelectedObject={selectedId !== null}
              onTool={setTool}
              onColor={setColor}
              onStroke={setStroke}
              onText={setText}
              onAdjust={(update) => setAdjustments((current) => ({ ...current, ...update }))}
              onUndo={history.undo}
              onRedo={history.redo}
              onDeleteObject={deleteObject}
              onAction={run}
              onTranslate={translate}
              onCancel={cancel}
              translateButtonRef={translateButtonRef}
            />
          )}
        </>
      )}
      {translation && selection && translationPosition && (
        <TranslationPopover
          state={translation}
          left={translationPosition.left}
          top={translationPosition.top}
          copyStatus={copyStatus}
          onCopy={() => void copyTranslation()}
          onClose={closeTranslation}
        />
      )}
      {!selection && !error && windowPickingUnavailable && (
        <div className="overlay-hint" role="status">{t("capture.windowPickingUnavailable")}</div>
      )}
      {error && <div className="overlay-error" role="status">{error}</div>}
    </main>
  );
}
