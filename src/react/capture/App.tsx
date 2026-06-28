import {
  ArrowUpRight,
  Copy,
  Crop,
  MousePointer2,
  PenLine,
  Pin,
  RectangleHorizontal,
  RotateCcw,
  Save,
  Type,
  Undo2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  clearPendingCapture,
  closeCurrentWindow,
  copyScreenshotImage,
  getPendingCapture,
  pinScreenshotImage,
  saveScreenshotImage,
  showCaptureEditor,
} from "../../js/api.js";
import {
  DEFAULT_IMAGE_ADJUSTMENTS,
  cssFilterForImageAdjustments,
  type ImageAdjustments,
} from "./imageAdjustments";
import type { Annotation, CapturedScreenshot, Point, Rect, Tool } from "./types";

type DragState =
  | { kind: "select"; start: Point }
  | { kind: "pen"; annotation: Annotation }
  | { kind: "rect"; start: Point; annotation: Annotation }
  | { kind: "arrow"; start: Point; annotation: Annotation }
  | null;

type Viewport = {
  width: number;
  height: number;
  fitScale: number;
  zoom: number;
  scale: number;
};

const MIN_ZOOM = 0.25;
const MAX_ZOOM = 6;
const MAX_CANVAS_DPR = 2;

const TOOL_OPTIONS: Array<{ id: Tool; label: string; icon: ReactNode }> = [
  { id: "select", label: "Select", icon: <MousePointer2 size={16} /> },
  { id: "pen", label: "Pen", icon: <PenLine size={16} /> },
  { id: "rect", label: "Rectangle", icon: <RectangleHorizontal size={16} /> },
  { id: "arrow", label: "Arrow", icon: <ArrowUpRight size={16} /> },
  { id: "text", label: "Text", icon: <Type size={16} /> },
];

const COLORS = ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff", "#111111"];

function makeId(prefix: string) {
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function normalizeRect(a: Point, b: Point): Rect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  };
}

function clampPoint(point: Point, image: HTMLImageElement): Point {
  return {
    x: Math.max(0, Math.min(image.naturalWidth, point.x)),
    y: Math.max(0, Math.min(image.naturalHeight, point.y)),
  };
}

function clampZoom(value: number) {
  return Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, value));
}

function shouldAppendPoint(points: Point[], point: Point) {
  const previous = points[points.length - 1];
  if (!previous) return true;
  const dx = point.x - previous.x;
  const dy = point.y - previous.y;
  return dx * dx + dy * dy >= 0.36;
}

function snapshotDraft(draft: DragState): DragState {
  if (!draft) return null;
  if (draft.kind === "pen" && draft.annotation.type === "pen") {
    return {
      ...draft,
      annotation: {
        ...draft.annotation,
        points: [...draft.annotation.points],
      },
    };
  }
  if (draft.kind === "rect" && draft.annotation.type === "rect") {
    return {
      ...draft,
      annotation: {
        ...draft.annotation,
        rect: { ...draft.annotation.rect },
      },
    };
  }
  if (draft.kind === "arrow" && draft.annotation.type === "arrow") {
    return {
      ...draft,
      annotation: {
        ...draft.annotation,
        from: { ...draft.annotation.from },
        to: { ...draft.annotation.to },
      },
    };
  }
  return draft;
}

function buildViewport(stage: HTMLElement, image: HTMLImageElement, zoom: number): Viewport {
  const maxWidth = Math.max(320, stage.clientWidth - 24);
  const maxHeight = Math.max(240, stage.clientHeight - 24);
  const fitScale = Math.min(maxWidth / image.naturalWidth, maxHeight / image.naturalHeight, 1);
  const safeZoom = clampZoom(zoom);
  const scale = Math.max(0.01, fitScale * safeZoom);
  return {
    width: Math.max(1, Math.round(image.naturalWidth * scale)),
    height: Math.max(1, Math.round(image.naturalHeight * scale)),
    fitScale,
    zoom: safeZoom,
    scale,
  };
}

function isExportSelection(selection: Rect | null): selection is Rect {
  return !!selection && selection.width >= 3 && selection.height >= 3;
}

function toPngBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error);
    reader.onload = () => {
      const result = String(reader.result || "");
      resolve(result.replace(/^data:image\/png;base64,/, ""));
    };
    reader.readAsDataURL(blob);
  });
}

function pngBase64ToObjectUrl(pngBase64: string): string {
  const binary = atob(pngBase64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

function drawArrowHead(ctx: CanvasRenderingContext2D, from: Point, to: Point, size: number) {
  const angle = Math.atan2(to.y - from.y, to.x - from.x);
  const length = Math.max(10, size * 4);
  ctx.beginPath();
  ctx.moveTo(to.x, to.y);
  ctx.lineTo(to.x - length * Math.cos(angle - Math.PI / 7), to.y - length * Math.sin(angle - Math.PI / 7));
  ctx.moveTo(to.x, to.y);
  ctx.lineTo(to.x - length * Math.cos(angle + Math.PI / 7), to.y - length * Math.sin(angle + Math.PI / 7));
  ctx.stroke();
}

function drawAnnotation(
  ctx: CanvasRenderingContext2D,
  annotation: Annotation,
  scale = 1,
  offset: Point = { x: 0, y: 0 },
) {
  ctx.save();
  ctx.strokeStyle = annotation.color;
  ctx.fillStyle = annotation.color;
  ctx.lineWidth = Math.max(1, annotation.size * scale);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  const p = (point: Point) => ({
    x: (point.x + offset.x) * scale,
    y: (point.y + offset.y) * scale,
  });

  if (annotation.type === "pen") {
    if (annotation.points.length < 2) {
      ctx.restore();
      return;
    }
    ctx.beginPath();
    const first = p(annotation.points[0]);
    ctx.moveTo(first.x, first.y);
    annotation.points.slice(1).forEach((point) => {
      const next = p(point);
      ctx.lineTo(next.x, next.y);
    });
    ctx.stroke();
  } else if (annotation.type === "rect") {
    const rect = annotation.rect;
    ctx.strokeRect(
      (rect.x + offset.x) * scale,
      (rect.y + offset.y) * scale,
      rect.width * scale,
      rect.height * scale,
    );
  } else if (annotation.type === "arrow") {
    const from = p(annotation.from);
    const to = p(annotation.to);
    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.stroke();
    drawArrowHead(ctx, from, to, annotation.size * scale);
  } else if (annotation.type === "text") {
    const at = p(annotation.at);
    const fontSize = Math.max(14, annotation.size * 4) * scale;
    ctx.font = `600 ${fontSize}px system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`;
    ctx.textBaseline = "top";
    ctx.lineWidth = Math.max(3, annotation.size * scale);
    ctx.strokeStyle = "rgba(0, 0, 0, 0.55)";
    ctx.strokeText(annotation.text, at.x, at.y);
    ctx.fillStyle = annotation.color;
    ctx.fillText(annotation.text, at.x, at.y);
  }

  ctx.restore();
}

function drawScene(
  canvas: HTMLCanvasElement,
  image: HTMLImageElement,
  viewport: Viewport,
  selection: Rect | null,
  annotations: Annotation[],
  draft: DragState,
  adjustments: ImageAdjustments,
) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const dpr = Math.min(MAX_CANVAS_DPR, Math.max(1, window.devicePixelRatio || 1));
  const pixelWidth = Math.max(1, Math.round(viewport.width * dpr));
  const pixelHeight = Math.max(1, Math.round(viewport.height * dpr));
  if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
    canvas.width = pixelWidth;
    canvas.height = pixelHeight;
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  ctx.clearRect(0, 0, viewport.width, viewport.height);
  ctx.save();
  ctx.filter = cssFilterForImageAdjustments(adjustments);
  ctx.drawImage(image, 0, 0, viewport.width, viewport.height);
  ctx.restore();

  annotations.forEach((annotation) => drawAnnotation(ctx, annotation, viewport.scale));
  if (draft && draft.kind !== "select") {
    drawAnnotation(ctx, draft.annotation, viewport.scale);
  }

  const activeSelection = selection || (draft?.kind === "select" ? normalizeRect(draft.start, draft.start) : null);
  if (activeSelection) {
    ctx.save();
    ctx.fillStyle = "rgba(0, 0, 0, 0.34)";
    ctx.fillRect(0, 0, viewport.width, viewport.height);
    ctx.clearRect(
      activeSelection.x * viewport.scale,
      activeSelection.y * viewport.scale,
      activeSelection.width * viewport.scale,
      activeSelection.height * viewport.scale,
    );
    ctx.strokeStyle = "#0a84ff";
    ctx.lineWidth = 2;
    ctx.setLineDash([8, 5]);
    ctx.strokeRect(
      activeSelection.x * viewport.scale,
      activeSelection.y * viewport.scale,
      activeSelection.width * viewport.scale,
      activeSelection.height * viewport.scale,
    );
    ctx.restore();
  }
}

export function App() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const imageObjectUrlRef = useRef<string | null>(null);
  const selectionRef = useRef<Rect | null>(null);
  const draftRef = useRef<DragState>(null);
  const zoomRef = useRef(1);
  const pendingSelectionRef = useRef<Rect | null>(null);
  const hasPendingSelectionRef = useRef(false);
  const hasPendingDraftRef = useRef(false);
  const pointerFrameRef = useRef<number | null>(null);
  const pendingZoomRef = useRef<number | null>(null);
  const wheelFrameRef = useRef<number | null>(null);
  const [capture, setCapture] = useState<CapturedScreenshot | null>(null);
  const [imageReady, setImageReady] = useState(false);
  const [viewport, setViewport] = useState<Viewport>({
    width: 1,
    height: 1,
    fitScale: 1,
    zoom: 1,
    scale: 1,
  });
  const [selection, setSelection] = useState<Rect | null>(null);
  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  const [draft, setDraft] = useState<DragState>(null);
  const [tool, setTool] = useState<Tool>("select");
  const [color, setColor] = useState(COLORS[0]);
  const [size, setSize] = useState(4);
  const [text, setText] = useState("Text");
  const [adjustments, setAdjustments] = useState<ImageAdjustments>(DEFAULT_IMAGE_ADJUSTMENTS);
  const [status, setStatus] = useState("Loading screenshot...");
  const [busy, setBusy] = useState(false);

  function flushPointerFrame() {
    if (hasPendingSelectionRef.current) {
      const nextSelection = pendingSelectionRef.current;
      hasPendingSelectionRef.current = false;
      pendingSelectionRef.current = null;
      selectionRef.current = nextSelection;
      setSelection(nextSelection);
    }
    if (hasPendingDraftRef.current) {
      const nextDraft = snapshotDraft(draftRef.current);
      hasPendingDraftRef.current = false;
      draftRef.current = nextDraft;
      setDraft(nextDraft);
    }
  }

  function cancelAndFlushPointerFrame() {
    if (pointerFrameRef.current !== null) {
      cancelAnimationFrame(pointerFrameRef.current);
      pointerFrameRef.current = null;
    }
    flushPointerFrame();
  }

  function schedulePointerFrame() {
    if (pointerFrameRef.current !== null) return;
    pointerFrameRef.current = requestAnimationFrame(() => {
      pointerFrameRef.current = null;
      flushPointerFrame();
    });
  }

  function scheduleSelection(nextSelection: Rect | null) {
    selectionRef.current = nextSelection;
    pendingSelectionRef.current = nextSelection;
    hasPendingSelectionRef.current = true;
    schedulePointerFrame();
  }

  function scheduleDraft(nextDraft: DragState) {
    draftRef.current = nextDraft;
    hasPendingDraftRef.current = true;
    schedulePointerFrame();
  }

  function setSelectionNow(nextSelection: Rect | null) {
    selectionRef.current = nextSelection;
    setSelection(nextSelection);
  }

  function setDraftNow(nextDraft: DragState) {
    draftRef.current = nextDraft;
    setDraft(nextDraft);
  }

  function releaseImageObjectUrl() {
    if (imageObjectUrlRef.current) {
      URL.revokeObjectURL(imageObjectUrlRef.current);
      imageObjectUrlRef.current = null;
    }
  }

  const activeImage = imageReady ? imageRef.current : null;
  const exportRect = useMemo(() => {
    if (!activeImage) return null;
    return isExportSelection(selection) ? selection : null;
  }, [activeImage, imageReady, selection]);
  const canExport = !!activeImage && !!exportRect && !busy;

  useEffect(() => {
    selectionRef.current = selection;
  }, [selection]);

  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  useEffect(() => {
    zoomRef.current = viewport.zoom;
  }, [viewport.zoom]);

  const loadPendingCapture = useCallback((cancelledRef?: { current: boolean }) => {
    getPendingCapture()
      .then((payload: CapturedScreenshot) => {
        if (cancelledRef?.current) return;
        setCapture(payload);
        setStatus("Ready");
      })
      .catch((err: unknown) => {
        if (cancelledRef?.current) return;
        console.error(err);
        setStatus("Failed to load screenshot");
      });
  }, []);

  useEffect(() => {
    const cancelledRef = { current: false };
    loadPendingCapture(cancelledRef);
    return () => {
      cancelledRef.current = true;
    };
  }, [loadPendingCapture]);

  useEffect(() => {
    return () => {
      if (pointerFrameRef.current !== null) {
        cancelAnimationFrame(pointerFrameRef.current);
        pointerFrameRef.current = null;
      }
      if (wheelFrameRef.current !== null) {
        cancelAnimationFrame(wheelFrameRef.current);
        wheelFrameRef.current = null;
      }
      const canvas = canvasRef.current;
      if (canvas) {
        canvas.width = 1;
        canvas.height = 1;
      }
      if (imageRef.current) {
        imageRef.current.src = "";
        imageRef.current = null;
      }
      releaseImageObjectUrl();
      void clearPendingCapture().catch(() => undefined);
    };
  }, []);

  const updateViewport = useCallback((zoomOverride?: number) => {
    const stage = stageRef.current;
    const image = imageRef.current;
    if (!stage || !image) return;
    setViewport((current) => buildViewport(stage, image, zoomOverride ?? current.zoom));
  }, []);

  useEffect(() => {
    if (!capture) return;
    setImageReady(false);
    if (imageRef.current) {
      imageRef.current.src = "";
      imageRef.current = null;
    }
    releaseImageObjectUrl();
    const image = new Image();
    let cancelled = false;
    let objectUrl: string;
    try {
      objectUrl = pngBase64ToObjectUrl(capture.pngBase64);
      imageObjectUrlRef.current = objectUrl;
    } catch (err) {
      console.error(err);
      setStatus("Failed to decode screenshot");
      setCapture(null);
      return;
    }
    image.onload = () => {
      if (cancelled) return;
      imageRef.current = image;
      setSelectionNow(null);
      setAnnotations([]);
      setDraftNow(null);
      setTool("select");
      setStatus("Select an area");
      setImageReady(true);
      updateViewport(1);
      setCapture(null);
    };
    image.onerror = () => {
      if (!cancelled) {
        setStatus("Failed to decode screenshot");
        setCapture(null);
      }
    };
    image.src = objectUrl;
    return () => {
      cancelled = true;
      image.onload = null;
      image.onerror = null;
      if (imageRef.current !== image) {
        image.src = "";
        if (imageObjectUrlRef.current === objectUrl) {
          releaseImageObjectUrl();
        } else {
          URL.revokeObjectURL(objectUrl);
        }
      }
    };
  }, [capture, updateViewport]);

  useEffect(() => {
    updateViewport();
    const stage = stageRef.current;
    if (!stage || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => updateViewport());
    observer.observe(stage);
    return () => observer.disconnect();
  }, [updateViewport]);

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;

    function onWheel(event: WheelEvent) {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      const baseZoom = pendingZoomRef.current ?? zoomRef.current;
      const nextZoom = clampZoom(baseZoom * Math.exp(-event.deltaY * 0.002));
      pendingZoomRef.current = nextZoom;
      zoomRef.current = nextZoom;
      if (wheelFrameRef.current !== null) return;
      wheelFrameRef.current = requestAnimationFrame(() => {
        wheelFrameRef.current = null;
        const zoom = pendingZoomRef.current;
        pendingZoomRef.current = null;
        if (zoom !== null) updateViewport(zoom);
      });
    }

    stage.addEventListener("wheel", onWheel, { passive: false });
    return () => stage.removeEventListener("wheel", onWheel);
  }, [updateViewport]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const image = imageRef.current;
    if (!canvas || !image) return;
    drawScene(canvas, image, viewport, selection, annotations, draft, adjustments);
  }, [viewport, selection, annotations, draft, adjustments, capture]);

  function pointFromEvent(event: React.PointerEvent<HTMLCanvasElement>): Point | null {
    const canvas = canvasRef.current;
    const image = imageRef.current;
    if (!canvas || !image || viewport.scale <= 0) return null;
    const rect = canvas.getBoundingClientRect();
    return clampPoint(
      {
        x: (event.clientX - rect.left) / viewport.scale,
        y: (event.clientY - rect.top) / viewport.scale,
      },
      image,
    );
  }

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    const point = pointFromEvent(event);
    const image = imageRef.current;
    if (!point || !image) return;
    event.currentTarget.setPointerCapture(event.pointerId);

    if (tool === "select") {
      setDraftNow({ kind: "select", start: point });
      setSelectionNow({ x: point.x, y: point.y, width: 0, height: 0 });
    } else if (tool === "pen") {
      setDraftNow({
        kind: "pen",
        annotation: { id: makeId("pen"), type: "pen", color, size, points: [point] },
      });
    } else if (tool === "rect") {
      setDraftNow({
        kind: "rect",
        start: point,
        annotation: {
          id: makeId("rect"),
          type: "rect",
          color,
          size,
          rect: { x: point.x, y: point.y, width: 0, height: 0 },
        },
      });
    } else if (tool === "arrow") {
      setDraftNow({
        kind: "arrow",
        start: point,
        annotation: { id: makeId("arrow"), type: "arrow", color, size, from: point, to: point },
      });
    } else if (tool === "text" && text.trim()) {
      setAnnotations((items) => [
        ...items,
        { id: makeId("text"), type: "text", color, size, at: point, text: text.trim() },
      ]);
    }
  }

  function onPointerMove(event: React.PointerEvent<HTMLCanvasElement>) {
    const point = pointFromEvent(event);
    const currentDraft = draftRef.current;
    if (!point || !currentDraft) return;

    if (currentDraft.kind === "select") {
      scheduleSelection(normalizeRect(currentDraft.start, point));
    } else if (currentDraft.kind === "pen" && currentDraft.annotation.type === "pen") {
      if (!shouldAppendPoint(currentDraft.annotation.points, point)) return;
      currentDraft.annotation.points.push(point);
      scheduleDraft(currentDraft);
    } else if (currentDraft.kind === "rect" && currentDraft.annotation.type === "rect") {
      scheduleDraft({
        ...currentDraft,
        annotation: {
          ...currentDraft.annotation,
          rect: normalizeRect(currentDraft.start, point),
        },
      });
    } else if (currentDraft.kind === "arrow" && currentDraft.annotation.type === "arrow") {
      scheduleDraft({
        ...currentDraft,
        annotation: {
          ...currentDraft.annotation,
          to: point,
        },
      });
    }
  }

  function onPointerUp() {
    cancelAndFlushPointerFrame();
    const currentDraft = draftRef.current;
    if (!currentDraft) return;
    if (currentDraft.kind === "select") {
      const currentSelection = selectionRef.current;
      setSelectionNow(
        currentSelection && currentSelection.width >= 3 && currentSelection.height >= 3
          ? currentSelection
          : null,
      );
    } else {
      const committedDraft = snapshotDraft(currentDraft);
      if (committedDraft && committedDraft.kind !== "select") {
        setAnnotations((items) => [...items, committedDraft.annotation]);
      }
    }
    setDraftNow(null);
  }

  async function exportPngBase64(): Promise<string> {
    const image = imageRef.current;
    if (!image) throw new Error("No screenshot loaded");
    if (!exportRect) throw new Error("Select an area first");
    const crop = exportRect;
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(crop.width));
    canvas.height = Math.max(1, Math.round(crop.height));
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas is not available");

    ctx.save();
    ctx.filter = cssFilterForImageAdjustments(adjustments);
    ctx.drawImage(
      image,
      crop.x,
      crop.y,
      crop.width,
      crop.height,
      0,
      0,
      canvas.width,
      canvas.height,
    );
    ctx.restore();
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, canvas.width, canvas.height);
    ctx.clip();
    annotations.forEach((annotation) => drawAnnotation(ctx, annotation, 1, { x: -crop.x, y: -crop.y }));
    ctx.restore();

    const blob = await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((result) => {
        if (result) resolve(result);
        else reject(new Error("Failed to export PNG"));
      }, "image/png");
    });
    return toPngBase64(blob);
  }

  async function runExport(action: "copy" | "save" | "pin") {
    if (!canExport) return;
    setBusy(true);
    setStatus(action === "copy" ? "Copying..." : action === "save" ? "Saving..." : "Pinning...");
    try {
      await new Promise((resolve) => requestAnimationFrame(resolve));
      const pngBase64 = await exportPngBase64();
      if (action === "copy") {
        await copyScreenshotImage(pngBase64);
        setStatus("Copied");
      } else if (action === "save") {
        const path = await saveScreenshotImage(pngBase64);
        setStatus(`Saved: ${path}`);
      } else {
        await pinScreenshotImage(pngBase64);
        setStatus("Pinned");
      }
    } catch (err) {
      console.error(err);
      setStatus(action === "copy" ? "Copy failed" : action === "save" ? "Save failed" : "Pin failed");
    } finally {
      setBusy(false);
    }
  }

  async function recapture() {
    setBusy(true);
    setStatus("Recapturing...");
    try {
      await showCaptureEditor();
      loadPendingCapture();
    } catch (err) {
      console.error(err);
      setStatus("Recapture failed");
    } finally {
      setBusy(false);
    }
  }

  function resetEdits() {
    cancelAndFlushPointerFrame();
    setSelectionNow(null);
    setAnnotations([]);
    setDraftNow(null);
    setTool("select");
    setAdjustments(DEFAULT_IMAGE_ADJUSTMENTS);
    setStatus("Select an area");
  }

  return (
    <main className="capture-app">
      <header className="capture-header">
        <div className="capture-title">
          <Crop size={18} />
          <span>Screenshot</span>
        </div>
        <div className="capture-actions">
          <button className="capture-btn" type="button" onClick={recapture} disabled={busy}>
            <RotateCcw size={16} />
            Recapture
          </button>
          <button className="capture-icon-btn" type="button" onClick={() => closeCurrentWindow()} title="Close">
            <X size={18} />
          </button>
        </div>
      </header>

      <section className="capture-body">
        <aside className="capture-sidebar">
          <div className="tool-group">
            <div className="tool-group-title">Tools</div>
            <div className="segmented-tools">
              {TOOL_OPTIONS.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  className={tool === option.id ? "tool-button active" : "tool-button"}
                  onClick={() => setTool(option.id)}
                  title={option.label}
                >
                  {option.icon}
                  <span>{option.label}</span>
                </button>
              ))}
            </div>
          </div>

          <div className="tool-group">
            <div className="tool-group-title">Style</div>
            <div className="color-row">
              {COLORS.map((item) => (
                <button
                  key={item}
                  type="button"
                  className={color === item ? "color-swatch active" : "color-swatch"}
                  style={{ backgroundColor: item }}
                  onClick={() => setColor(item)}
                  title={item}
                />
              ))}
            </div>
            <label className="control-row">
              <span>Size</span>
              <input
                type="range"
                min="2"
                max="16"
                value={size}
                onChange={(event) => setSize(Number(event.target.value))}
              />
              <output>{size}</output>
            </label>
            <label className="text-control">
              <span>Text</span>
              <input value={text} onChange={(event) => setText(event.target.value)} />
            </label>
          </div>

          <div className="tool-group">
            <div className="tool-group-title">Image</div>
            <label className="toggle-row">
              <input
                type="checkbox"
                checked={adjustments.grayscale}
                onChange={(event) =>
                  setAdjustments((current) => ({ ...current, grayscale: event.target.checked }))
                }
              />
              <span>Grayscale</span>
            </label>
            {(["brightness", "contrast", "saturation"] as const).map((key) => (
              <label className="control-row" key={key}>
                <span>{key[0].toUpperCase() + key.slice(1)}</span>
                <input
                  type="range"
                  min="-100"
                  max="100"
                  value={adjustments[key]}
                  onChange={(event) =>
                    setAdjustments((current) => ({ ...current, [key]: Number(event.target.value) }))
                  }
                />
                <output>{adjustments[key]}</output>
              </label>
            ))}
          </div>
        </aside>

        <section className="capture-stage-shell">
          <div className="capture-stage" ref={stageRef}>
            <canvas
              ref={canvasRef}
              className={`capture-canvas capture-canvas-${tool}`}
              style={{ width: `${viewport.width}px`, height: `${viewport.height}px` }}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerUp}
            />
          </div>
          <footer className="capture-status">
            <span>{status}</span>
            <span>
              {exportRect
                ? `${Math.round(exportRect.width)} x ${Math.round(exportRect.height)}`
                : activeImage
                  ? "No selection"
                  : "No image"}
              {" · "}
              {Math.round(viewport.zoom * 100)}%
            </span>
          </footer>
        </section>
      </section>

      <footer className="capture-footer">
        <button className="capture-btn" type="button" onClick={() => setAnnotations((items) => items.slice(0, -1))} disabled={annotations.length === 0 || busy}>
          <Undo2 size={16} />
          Undo
        </button>
        <button className="capture-btn" type="button" onClick={resetEdits} disabled={busy}>
          <RotateCcw size={16} />
          Reset
        </button>
        <div className="capture-footer-spacer" />
        <button className="capture-btn primary" type="button" onClick={() => runExport("copy")} disabled={!canExport}>
          <Copy size={16} />
          Copy
        </button>
        <button className="capture-btn" type="button" onClick={() => runExport("save")} disabled={!canExport}>
          <Save size={16} />
          Save
        </button>
        <button className="capture-btn" type="button" onClick={() => runExport("pin")} disabled={!canExport}>
          <Pin size={16} />
          Pin
        </button>
      </footer>
    </main>
  );
}
