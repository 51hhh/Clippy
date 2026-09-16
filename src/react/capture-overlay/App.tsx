import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { CaptureOutputError } from "../../js/ipc-types.ts";
import { getCurrentWindowLabel } from "../../js/api.ts";
import { drawScene } from "../annotation/canvasRenderer";
import { type FrameImage, paintRgbaFrame, rgbaToFrameCanvas } from "../annotation/frameImage";
import {
  DEFAULT_IMAGE_ADJUSTMENTS,
  hasImageAdjustments,
  type ImageAdjustments,
} from "../annotation/imageAdjustments";
import type { Annotation, Tool } from "../annotation/types";
import { useCanvasInteractions } from "../annotation/useCanvasInteractions";
import { useHistory } from "../annotation/useHistory";
import { t } from "../shared/i18n";
import { overlayApi } from "./api";
import { OverlayToolbar } from "./OverlayToolbar";
import { OutputFailurePanel } from "./OutputFailurePanel";
import { DEFAULT_COLOR, DEFAULT_STROKE } from "./tools";
import { TranslationPopover } from "./TranslationPopover";
import {
  isCurrentTranslation,
  translationErrorMessage,
  translationPanelPosition,
} from "./translationState";
import type {
  CaptureAction,
  CaptureOrigin,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationState,
  OverlayTool,
  Point,
  Rect,
  ResizeHandle,
} from "./types";
import { useSelection } from "./useSelection";
import { toPixelRect } from "./geometry";

const HANDLES: ResizeHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
type LongshotLaunchState = "idle" | "pending" | "accepted";

/** 普通覆盖层只做一次轻量预检，实际会话/模式权威仍在后端。 */
function longshotSelection(
  payload: CaptureOverlayPayload | null,
  selection: Rect | null,
): CaptureSelection | null {
  if (!payload || !selection) return null;
  const geometry = [
    payload.logicalWidth,
    payload.logicalHeight,
    selection.x,
    selection.y,
    selection.width,
    selection.height,
  ];
  if (
    !payload.sessionId ||
    !Number.isFinite(payload.monitorId) ||
    geometry.some((value) => !Number.isFinite(value)) ||
    payload.logicalWidth < 2 ||
    payload.logicalHeight < 2 ||
    selection.x < 0 ||
    selection.y < 0 ||
    selection.width < 2 ||
    selection.height < 2 ||
    selection.x + selection.width > payload.logicalWidth ||
    selection.y + selection.height > payload.logicalHeight
  ) {
    return null;
  }
  const pixels = toPixelRect(selection, payload.pixelWidth / payload.logicalWidth,
    payload.pixelHeight / payload.logicalHeight, { x: 0, y: 0, width: payload.pixelWidth, height: payload.pixelHeight });
  if (!Number.isFinite(pixels.width) || !Number.isFinite(pixels.height) || pixels.width < 8 || pixels.height < 24) return null;
  return { ...selection, sessionId: payload.sessionId, monitorId: payload.monitorId };
}

/** 覆盖层窗口的可见视口尺寸（CSS 像素）。合成器最终摆放的尺寸只有这里能看到。 */
function useViewportSize() {
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    function measure() {
      setSize({ width: window.innerWidth, height: window.innerHeight });
    }
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);
  return size;
}

/**
 * 冻结画面覆盖层：选区、标注和提交全部发生在这一个窗口里。
 *
 * 选区交互由选区和当前工具推导，不额外存：
 * - idle：没有选区。悬停窗口高亮可速选，点一下取悬停窗口、点空地取整屏。
 * - dragging：按住不放，正在框选 / 移动 / 缩放选区，或正在画一笔标注。
 * - editing：有选区。工具条贴在选区旁边，选区仍可拖动与缩放，
 *   点对钩就把"裁剪 + 标注"后的 PNG 直接送进剪贴板。
 * 输出期间锁定编辑；后端确认渲染失败才恢复编辑，产物输出失败进入独立恢复面板。
 */
/** 同一次 mount 的服务保持稳定；生产入口省略该参数。 */
export type CaptureAppServices = { api: typeof overlayApi; windowLabel: () => string };
const defaultServices: CaptureAppServices = { api: overlayApi, windowLabel: getCurrentWindowLabel };

export function App({ services = defaultServices }: { services?: CaptureAppServices } = {}) {
  const overlayApi = services.api;
  const label = services.windowLabel();
  const [payload, setPayload] = useState<CaptureOverlayPayload | null>(null);
  const [frameBuffer, setFrameBuffer] = useState<ArrayBuffer | null>(null);
  const [frameProtocolFailed, setFrameProtocolFailed] = useState(false);
  const [imageReady, setImageReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [outputFailure, setOutputFailure] = useState<CaptureOutputError | null>(null);
  const outputFailureRef = useRef<CaptureOutputError | null>(null);
  const handoffLabel = useRef<string | null>(null);
  const handoffReadyRef = useRef(false);
  const [handoffReady, setHandoffReady] = useState(false);
  const handoffResults = useRef(new Map<string, boolean>());
  const [tool, setTool] = useState<OverlayTool>("select");
  const [color, setColor] = useState(DEFAULT_COLOR);
  const [stroke, setStroke] = useState(DEFAULT_STROKE);
  const [text, setText] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [adjustments, setAdjustments] = useState<ImageAdjustments>(DEFAULT_IMAGE_ADJUSTMENTS);
  const [translation, setTranslation] = useState<CaptureTranslationState | null>(null);
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);
  const translationGeneration = useRef(0);
  const translationBusyRef = useRef(false);
  const [longshotLaunch, setLongshotLaunch] = useState<LongshotLaunchState>("idle");
  const longshotLaunchRef = useRef<LongshotLaunchState>("idle");
  const mountedEpoch = useRef(0);
  const translateButtonRef = useRef<HTMLButtonElement>(null);
  const imageRef = useRef<FrameImage | null>(null);
  const frameHostRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const activeDrag = useRef<"selection" | "canvas" | null>(null);
  const revealed = useRef(false);

  useEffect(() => {
    const epoch = mountedEpoch.current + 1;
    mountedEpoch.current = epoch;
    return () => {
      if (mountedEpoch.current === epoch) mountedEpoch.current += 1;
    };
  }, []);

  /**
   * 让后端把覆盖层显示出来。窗口是隐藏建窗的：加载 webview、取 payload 与底图的
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
  const viewport = useViewportSize();
  /**
   * 工具条与译文面板按**真实可见视口**落位，而不是冻结帧的逻辑尺寸。
   *
   * 正常情况下两者相等（覆盖层就是铺满那块屏）。不相等只发生在冻结帧几何算错时——
   * 曾经多屏混合缩放下帧几何被算大 1.125 倍（见 `screenshot/backends.rs` 的
   * `desktop_max_scale_factor`），画布比窗口大一圈，贴在选区右下的工具条就落到了
   * 窗口外面，用户看到的是"截图能用但工具条不见了"。按可见视口钳一下，
   * 这类不一致只会让画布边缘被裁掉，交互仍然完整。
   */
  const layoutWidth = viewport.width > 0 ? Math.min(logicalWidth, viewport.width) : logicalWidth;
  const layoutHeight = viewport.height > 0 ? Math.min(logicalHeight, viewport.height) : logicalHeight;
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

  // 首选 WebKit 原生资源管线，避免把双屏约 50 MB RGBA 穿过 JS invoke 桥。
  // 图像协议仍是无损原始像素封装；只有协议加载失败才退回旧的 ArrayBuffer 路径。
  useEffect(() => {
    if (!payload) return;
    let cancelled = false;
    overlayApi
      .image(label)
      .then((image) => {
        if (cancelled) return;
        if (image.naturalWidth !== payload.pixelWidth || image.naturalHeight !== payload.pixelHeight) {
          throw new Error(
            `capture frame size mismatch: ${image.naturalWidth}x${image.naturalHeight}`,
          );
        }
        image.className = "overlay-frame";
        image.draggable = false;
        image.setAttribute("aria-hidden", "true");
        frameHostRef.current?.replaceChildren(image);
        imageRef.current = image;
        setImageReady(true);
      })
      .catch((reason) => {
        if (cancelled) return;
        console.warn("截图帧协议加载失败，回退到二进制 IPC", reason);
        setFrameProtocolFailed(true);
      });
    return () => {
      cancelled = true;
      frameHostRef.current?.replaceChildren();
      imageRef.current = null;
      setImageReady(false);
    };
  }, [label, payload]);

  useEffect(() => {
    if (!frameProtocolFailed) return;
    let cancelled = false;
    overlayApi
      .frame(label)
      .then((buffer) => !cancelled && setFrameBuffer(buffer))
      .catch((reason) => {
        if (cancelled) return;
        console.warn("截图冻结帧读取失败", reason);
        setError(t("capture.decodeFailed"));
      });
    return () => {
      cancelled = true;
      setFrameBuffer(null);
    };
  }, [frameProtocolFailed, label]);

  useEffect(() => {
    if (!payload || !frameBuffer || !frameProtocolFailed) return;
    const target = canvasRef.current;
    if (!target) return;
    try {
      // 首帧直接写最终画布。旧路径先创建同尺寸离屏画布，再整屏 drawImage 一次；
      // 双屏 4K 下会多占几十 MB，并把窗口显示推迟一整次全图合成。
      paintRgbaFrame(
        target,
        new Uint8ClampedArray(frameBuffer),
        payload.pixelWidth,
        payload.pixelHeight,
      );
      imageRef.current = target;
      setImageReady(true);
      reveal();
    } catch (reason) {
      console.warn("截图冻结帧解码失败", reason);
      setError(t("capture.decodeFailed"));
    }
    return () => {
      imageRef.current = null;
      setImageReady(false);
    };
  }, [frameBuffer, frameProtocolFailed, payload, reveal]);

  useEffect(() => {
    // 排障线索：窗口几何拿不到时界面只有一句提示，日志里要留下原因可查。
    if (payload && payload.windows.length === 0) {
      console.warn("截图覆盖层未拿到窗口几何，窗口速选不可用");
    }
  }, [payload]);

  const selection = region.selection;
  // `crop` 草稿不是注解（选区由 useSelection 管），画布只关心能画出来的那几种。
  const draftAnnotation = canvas.draft && "annotation" in canvas.draft
    ? canvas.draft.annotation
    : null;
  const roundedCrop = useMemo(() => adjustments.cornerRadius > 0 && selection && payload
    ? toPixelRect(selection, payload.pixelWidth / logicalWidth, payload.pixelHeight / logicalHeight,
      { x: 0, y: 0, width: payload.pixelWidth, height: payload.pixelHeight })
    : null, [adjustments.cornerRadius, selection, payload, logicalWidth, logicalHeight]);
  const needsComposite =
    annotations.length > 0 ||
    draftAnnotation !== null ||
    selectedId !== null ||
    hasImageAdjustments(adjustments) ||
    adjustments.cornerRadius > 0;
  const longshotDirty =
    annotations.length > 0 ||
    draftAnnotation !== null ||
    hasImageAdjustments(adjustments) ||
    adjustments.cornerRadius !== 0;

  // 底图与标注画在这块画布上，因此标注相关的状态一变就要重绘。
  //
  // **普通选区不在依赖里，这是有意的；启用圆角时 roundedCrop 需要随选区更新。** 画一帧要把 2560×1600 的冻结帧高质量缩绘进
  // 1920×1200 的画布，而拖动/缩放选区时每个 pointermove 都会改选区矩形——压暗和
  // 虚线框留在画布上的话，等于每帧白做一次全图重采样。它们现在由 `.selection` 的
  // `outline` + `box-shadow` 画（overlay.css），合成器代价近似为零，
  // 于是"拖选区"这条最高频的交互引发的画布重绘次数是 0。
  useLayoutEffect(() => {
    const target = canvasRef.current;
    let image = imageRef.current;
    if (!target || !image || !imageReady) return;
    // 原生协议已经把无损冻结帧解成 <img>；未编辑时直接显示它，不再为了首帧额外分配并
    // 填满一块同尺寸 Canvas。用户开始标注/调色后才走下面的合成路径。
    if (image !== target && !needsComposite && !frameProtocolFailed) {
      reveal();
      return;
    }
    // 原始 RGBA 兜底已经把首帧直接写进最终 Canvas，不要紧接着再复制、再重画。
    if (image === target && !needsComposite) return;
    if (image === target) {
      if (!payload || !frameBuffer) return;
      image = rgbaToFrameCanvas(
        new Uint8ClampedArray(frameBuffer),
        payload.pixelWidth,
        payload.pixelHeight,
      );
      imageRef.current = image;
    }
    drawScene(
      target,
      image,
      {
        width: logicalWidth,
        height: logicalHeight,
        fitScale: scale,
        zoom: 1,
        scale,
        // 冻结帧已经按显示器实际缩放抓取；画布保持同样的物理像素尺寸，
        // 避免 WebKit 的整数 DPR 在混合缩放下把它再次放大后交给 Mutter 缩回。
        pixelRatio: 1 / scale,
      },
      annotations,
      draftAnnotation,
      adjustments,
      selectedId,
      roundedCrop,
    );
    // 首帧已经落在画布上，可以显示窗口了。
    reveal();
  }, [
    adjustments,
    annotations,
    draftAnnotation,
    frameBuffer,
    frameProtocolFailed,
    imageReady,
    logicalHeight,
    logicalWidth,
    payload,
    reveal,
    scale,
    selectedId,
    needsComposite,
    roundedCrop,
  ]);

  // 出错时也要把窗口显示出来，否则用户只看到截图"没反应"，错误提示压根没露面。
  useEffect(() => {
    if (error) reveal();
  }, [error, reveal]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    handoffReadyRef.current = false;
    setHandoffReady(false);
    void overlayApi.onHandoff((result) => {
      if (result.sessionId !== payload?.sessionId) return;
      handoffResults.current.set(result.controllerLabel, result.accepted);
      if (handoffLabel.current !== result.controllerLabel) return;
      longshotLaunchRef.current = result.accepted ? "accepted" : "idle";
      setLongshotLaunch(longshotLaunchRef.current);
      if (!result.accepted) setError(t("capture.longshotOpenFailed"));
    }).then((stop) => {
      if (cancelled) { stop(); return; }
      unlisten = stop;
      handoffReadyRef.current = true;
      setHandoffReady(true);
    }).catch((reason) => {
      console.warn(reason);
      if (!cancelled) setError(t("capture.longshotOpenFailed"));
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [payload?.sessionId]);

  const handleOutputError = useCallback((reason: unknown, retrying = false) => {
    const candidate = reason && typeof reason === "object" ? reason as Partial<CaptureOutputError> : null;
    const structured = typeof candidate?.outputPending === "boolean" && typeof candidate.message === "string"
      && Array.isArray(candidate.retryActions)
      && candidate.retryActions.every((action) => action === "copy" || action === "save" || action === "pin");
    // IPC 回包丢失不能证明输出尚未执行；只允许幂等复制，后端仍检查产物与认领状态。
    let failure: CaptureOutputError = structured ? candidate as CaptureOutputError
      : { message: String(reason), outputPending: true, retryActions: ["copy"] };
    const previous = outputFailureRef.current;
    if (retrying && previous) {
      // 恢复请求被拒绝不代表旧输出已结束；只能保持或收紧权限，不能退回编辑。
      failure = { ...failure, outputPending: true, retryActions: failure.outputPending
        ? previous.retryActions.filter((action) => failure.retryActions.includes(action))
        : previous.retryActions };
    }
    outputFailureRef.current = failure.outputPending ? failure : null;
    setOutputFailure(outputFailureRef.current);
    setError(failure.message);
    busyRef.current = false;
    setBusy(false);
  }, []);

  const retryOutput = useCallback((action: CaptureAction) => {
    if (busyRef.current || !outputFailureRef.current?.retryActions.includes(action)) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    void overlayApi.retry(action).catch((reason) => handleOutputError(reason, true));
  }, [handleOutputError]);

  const cancel = useCallback(() => {
    if (busyRef.current || longshotLaunchRef.current === "pending") return;
    translationGeneration.current += 1;
    translationBusyRef.current = false;
    setTranslation(null);
    setCopyStatus(null);
    busyRef.current = true;
    setBusy(true);
    const operation = payload ? overlayApi.cancel(payload.sessionId) : overlayApi.closeUninitialized();
    operation.catch((reason) => {
      setError(String(reason));
      busyRef.current = false;
      setBusy(false);
    });
  }, [payload]);

  /**
   * 选区在**桌面**逻辑坐标里的位置。覆盖层坐标是相对自己这块屏幕的，加上
   * payload 的 logicalX/logicalY 才是全局坐标——贴图靠它回到截图时的原位。
   */
  const originRect = useMemo<CaptureOrigin | null>(
    () =>
      payload && selection
        ? {
          x: payload.logicalX + selection.x,
          y: payload.logicalY + selection.y,
          width: selection.width,
          height: selection.height,
        }
        : null,
    [payload, selection],
  );

  /** 提交 v2 操作层；后端从会话冻结帧合成并裁出权威 PNG。 */
  const run = useCallback(
    (action: CaptureAction) => {
      const image = imageRef.current;
      if (outputFailureRef.current) return;
      if (
        !payload ||
        !image ||
        !selection ||
        busyRef.current ||
        translationBusyRef.current ||
        longshotLaunchRef.current === "pending"
      ) return;
      translationGeneration.current += 1;
      translationBusyRef.current = false;
      setTranslation(null);
      busyRef.current = true;
      setBusy(true);
      setError(null);
      overlayApi
        .commit(
          action,
          { ...selection, sessionId: payload.sessionId, monitorId: payload.monitorId },
          {
            rendererVersion: 2,
            sourceWidth: payload.pixelWidth,
            sourceHeight: payload.pixelHeight,
            annotations,
            adjustments,
          },
          originRect,
        )
        .catch(handleOutputError);
    },
    [adjustments, annotations, handleOutputError, originRect, payload, selection],
  );

  const closeTranslation = useCallback(() => {
    if (busyRef.current || outputFailureRef.current || longshotLaunchRef.current === "pending") return;
    translationGeneration.current += 1;
    translationBusyRef.current = false;
    setTranslation(null);
    setCopyStatus(null);
    requestAnimationFrame(() => translateButtonRef.current?.focus());
  }, []);

  const translate = useCallback(() => {
    if (
      !payload ||
      !selection ||
      busyRef.current ||
      translationBusyRef.current ||
      longshotLaunchRef.current === "pending"
    ) return;
    const generation = translationGeneration.current + 1;
    translationGeneration.current = generation;
    translationBusyRef.current = true;
    setCopyStatus(null);
    setTranslation({ status: "loading" });
    overlayApi
      .translate({ ...selection, sessionId: payload.sessionId, monitorId: payload.monitorId })
      .then((result) => {
        if (isCurrentTranslation(translationGeneration.current, generation)) {
          translationBusyRef.current = false;
          setTranslation({ status: "result", result });
        }
      })
      .catch((reason) => {
        if (isCurrentTranslation(translationGeneration.current, generation)) {
          translationBusyRef.current = false;
          setTranslation({ status: "error", message: translationErrorMessage(reason) });
        }
      });
  }, [payload, selection]);

  const copyTranslation = useCallback(async () => {
    if (translation?.status !== "result" || longshotLaunchRef.current === "pending") return;
    try {
      await overlayApi.copyText(translation.result.translatedText);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("failed");
    }
  }, [translation]);

  const deleteObject = useCallback(() => {
    if (!selectedId || busyRef.current || outputFailureRef.current || longshotLaunchRef.current === "pending") return;
    history.commit((items) => items.filter((item) => item.id !== selectedId));
    setSelectedId(null);
  }, [history, selectedId]);

  const openLongshot = useCallback(() => {
    if (
      !handoffReadyRef.current || outputFailureRef.current ||
      longshotLaunchRef.current !== "idle" ||
      longshotDirty ||
      busyRef.current ||
      translationBusyRef.current
    ) return;
    const candidate = longshotSelection(payload, selection);
    if (!candidate) return;

    // React state 到下一轮渲染才可见；先写 ref 才能挡住同 tick 的双击与键盘事件。
    handoffLabel.current = null;
    handoffResults.current.clear();
    longshotLaunchRef.current = "pending";
    setLongshotLaunch("pending");
    translationGeneration.current += 1;
    translationBusyRef.current = false;
    setTranslation(null);
    setCopyStatus(null);
    setError(null);
    const epoch = mountedEpoch.current;

    overlayApi.openLongshot(candidate).then(
      (result) => {
        if (mountedEpoch.current !== epoch) return;
        handoffLabel.current = result.label;
        const accepted = handoffResults.current.get(result.label);
        longshotLaunchRef.current = accepted === false ? "idle" : "accepted";
        setLongshotLaunch(longshotLaunchRef.current);
        if (accepted === false) setError(t("capture.longshotOpenFailed"));
      },
      (reason) => {
        if (mountedEpoch.current !== epoch) return;
        console.warn("长截图控制窗口创建失败", reason);
        longshotLaunchRef.current = "idle";
        setLongshotLaunch("idle");
        setError(t("capture.longshotOpenFailed"));
      },
    );
  }, [longshotDirty, payload, selection]);

  const setToolIfEditable = useCallback((next: OverlayTool) => {
    if (!busyRef.current && !outputFailureRef.current && longshotLaunchRef.current !== "pending") setTool(next);
  }, []);
  const setColorIfEditable = useCallback((next: string) => {
    if (!busyRef.current && !outputFailureRef.current && longshotLaunchRef.current !== "pending") setColor(next);
  }, []);
  const setStrokeIfEditable = useCallback((next: number) => {
    if (!busyRef.current && !outputFailureRef.current && longshotLaunchRef.current !== "pending") setStroke(next);
  }, []);
  const setTextIfEditable = useCallback((next: string) => {
    if (!busyRef.current && !outputFailureRef.current && longshotLaunchRef.current !== "pending") setText(next);
  }, []);
  const adjustIfEditable = useCallback((update: Partial<ImageAdjustments>) => {
    if (!busyRef.current && !outputFailureRef.current && longshotLaunchRef.current !== "pending") {
      setAdjustments((current) => ({ ...current, ...update }));
    }
  }, []);
  const undoIfEditable = useCallback(() => {
    if (!busyRef.current && !outputFailureRef.current && longshotLaunchRef.current !== "pending") history.undo();
  }, [history]);
  const redoIfEditable = useCallback(() => {
    if (!busyRef.current && !outputFailureRef.current && longshotLaunchRef.current !== "pending") history.redo();
  }, [history]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (longshotLaunchRef.current === "pending") {
        event.preventDefault();
        return;
      }
      const typing = event.target instanceof HTMLElement && event.target.tagName === "INPUT";
      if (event.key === "Escape") {
        event.preventDefault();
        if (translation) closeTranslation();
        else cancel();
        return;
      }
      if (typing || busyRef.current || outputFailureRef.current) return;
      const control = event.ctrlKey || event.metaKey;
      if (control && event.key.toLowerCase() === "z") {
        event.preventDefault();
        if (event.shiftKey) redoIfEditable();
        else undoIfEditable();
      } else if (control && event.key.toLowerCase() === "y") {
        event.preventDefault();
        redoIfEditable();
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
  }, [cancel, closeTranslation, deleteObject, redoIfEditable, run, selectedId, selection, translation, undoIfEditable]);

  function point(event: React.PointerEvent): Point {
    return { x: event.clientX, y: event.clientY };
  }

  /**
   * 指针路由：缩放手柄永远归选区（这样标注工具激活时依旧能改选区），
   * 其余情况下绘制工具归画布、`select` 工具归选区。
   */
  function onPointerDown(event: React.PointerEvent) {
    if (busyRef.current || outputFailureRef.current || longshotLaunchRef.current === "pending") return;
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
    if (busyRef.current || outputFailureRef.current || longshotLaunchRef.current === "pending") return;
    if (activeDrag.current === "canvas") canvas.onPointerMove(event);
    else region.pointerMove(point(event));
  }

  function onPointerUp(event: React.PointerEvent) {
    if (busyRef.current || outputFailureRef.current || longshotLaunchRef.current === "pending") return;
    const owner = activeDrag.current;
    activeDrag.current = null;
    if (owner === "canvas") canvas.onPointerUp();
    else region.pointerUp(point(event));
  }

  if (!payload) {
    return (
      <main className="overlay-root loading">
        {error && <div className="overlay-error" role="alert">{error}
          <button type="button" disabled={busy} onClick={cancel}>{t("capture.cancel")}</button>
        </div>}
      </main>
    );
  }
  // 选区外面继续给窗口预览，速选不会因为框过一次就消失；
  // 但标注工具激活时不给高亮，否则画笔在选区外扫一下就闪一个蓝框。
  const preview = tool === "select" ? region.candidate : null;
  // 拿不到窗口列表（部分 Wayland 合成器不给窗口几何）时明说，否则用户只会觉得速选坏了。
  // GNOME Wayland 上这事是可以解决的（装个 Shell 扩展），后端在首次遇到时置 probeHint，
  // 这时候给出能照着做的说法，而不是只说"用不了"。
  const showHint = payload.windows.length === 0 || payload.probeHint;
  const hintKey = payload.probeHint
    ? "capture.windowProbeHint"
    : "capture.windowPickingUnavailable";
  const translationPosition = selection && translation
    ? translationPanelPosition(selection, layoutWidth, layoutHeight)
    : null;
  const validLongshotSelection = longshotSelection(payload, selection) !== null;
  const longshotActionBusy = busyRef.current || translationBusyRef.current;
  const longshotDisabledReason = longshotLaunch === "pending"
    ? t("capture.longshotPending")
    : longshotLaunch === "accepted"
      ? t("capture.longshotAccepted")
      : longshotDirty
        ? t("capture.longshotDirty")
        : longshotActionBusy
          ? t("capture.longshotBusy")
        : t("capture.longshotInvalid");

  return (
    <main
      className="overlay-root"
      data-tool={tool}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onContextMenu={(event) => {
        event.preventDefault();
        if (busyRef.current || outputFailureRef.current || longshotLaunchRef.current === "pending") return;
        // 右键回到"还没框选"，这样点一次取了整屏之后还能重新框小区域。
        region.reset();
        setSelectedId(null);
      }}
    >
      <div
        ref={frameHostRef}
        className={`overlay-frame-host${needsComposite ? " is-composited" : ""}`}
      />
      <canvas
        ref={canvasRef}
        className={`overlay-canvas${!frameProtocolFailed && !needsComposite ? " is-idle" : ""}`}
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
            onPointerDown={(event) => { event.preventDefault(); event.stopPropagation(); }}
            onPointerMove={(event) => event.stopPropagation()}
            onPointerUp={(event) => event.stopPropagation()}
            onPointerCancel={(event) => event.stopPropagation()}
            style={{ left: selection.x, top: Math.max(6, selection.y - 28) }}
          >
            {Math.round(selection.width)} × {Math.round(selection.height)}
          </div>
          {selection.width >= 2 && selection.height >= 2 && (
            <OverlayToolbar
              selection={selection}
              viewportWidth={layoutWidth}
              viewportHeight={layoutHeight}
              tool={tool}
              color={color}
              stroke={stroke}
              text={text}
              adjustments={adjustments}
              busy={busy || outputFailure !== null}
              translationBusy={translation?.status === "loading"}
              longshotPending={longshotLaunch === "pending"}
              longshotDisabled={
                !handoffReady || longshotLaunch !== "idle" ||
                longshotDirty ||
                longshotActionBusy ||
                !validLongshotSelection
              }
              longshotDisabledReason={longshotDisabledReason}
              canUndo={history.canUndo}
              canRedo={history.canRedo}
              hasSelectedObject={selectedId !== null}
              onTool={setToolIfEditable}
              onColor={setColorIfEditable}
              onStroke={setStrokeIfEditable}
              onText={setTextIfEditable}
              onAdjust={adjustIfEditable}
              onUndo={undoIfEditable}
              onRedo={redoIfEditable}
              onDeleteObject={deleteObject}
              onAction={run}
              onTranslate={translate}
              onLongshot={openLongshot}
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
      {!selection && !error && showHint && (
        <div className="overlay-hint" role="status">{t(hintKey)}</div>
      )}
      {outputFailure ? <OutputFailurePanel failure={outputFailure} message={error} busy={busy}
        onRetry={retryOutput} onDiscard={cancel} /> : error && <div className="overlay-error" role="status">{error}</div>}
    </main>
  );
}
