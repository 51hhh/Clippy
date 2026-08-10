import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  clearPendingCapture,
  closeCurrentWindow,
  copyScreenshotImage,
  getPendingCapture,
  onCaptureLoaded,
  pinScreenshotImage,
  saveScreenshotImage,
  showCaptureEditor,
} from "../../js/api.js";
import {
  DEFAULT_IMAGE_ADJUSTMENTS,
} from "./imageAdjustments";
import { drawScene, renderExport, type RenderViewport } from "./canvasRenderer";
import { EditorFooter, EditorHeader } from "./EditorChrome";
import { EditorSidebar } from "./EditorSidebar";
import { useCanvasInteractions } from "./useCanvasInteractions";
import { useHistory } from "./useHistory";
import type { Annotation, CapturedScreenshot, EditorDocument, Rect, Tool } from "./types";

const MIN_ZOOM = 0.25;
const MAX_ZOOM = 6;

const DEFAULT_COLOR = "#ff3b30";

function clampZoom(value: number) {
  return Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, value));
}

function buildViewport(stage: HTMLElement, image: HTMLImageElement, zoom: number): RenderViewport {
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

export function App() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const imageObjectUrlRef = useRef<string | null>(null);
  const zoomRef = useRef(1);
  const pendingZoomRef = useRef<number | null>(null);
  const wheelFrameRef = useRef<number | null>(null);
  const [capture, setCapture] = useState<CapturedScreenshot | null>(null);
  const [imageReady, setImageReady] = useState(false);
  const [viewport, setViewport] = useState<RenderViewport>({
    width: 1,
    height: 1,
    fitScale: 1,
    zoom: 1,
    scale: 1,
  });
  const [selection, setSelection] = useState<Rect | null>(null);
  const [tool, setTool] = useState<Tool>("object");
  const [selectedAnnotationId, setSelectedAnnotationId] = useState<string | null>(null);
  const [color, setColor] = useState(DEFAULT_COLOR);
  const [size, setSize] = useState(4);
  const [text, setText] = useState("Text");
  const [status, setStatus] = useState("Loading screenshot...");
  const [busy, setBusy] = useState(false);
  const {
    value: editorDocument,
    canUndo,
    canRedo,
    commit: commitDocument,
    undo,
    redo,
    reset: resetDocument,
  } = useHistory<EditorDocument>({
    annotations: [],
    adjustments: DEFAULT_IMAGE_ADJUSTMENTS,
  });
  const { annotations, adjustments } = editorDocument;

  function commitAnnotations(update: Annotation[] | ((items: Annotation[]) => Annotation[])) {
    commitDocument((current) => ({
      ...current,
      annotations: typeof update === "function" ? update(current.annotations) : update,
    }));
  }

  function commitAdjustments(update: Partial<EditorDocument["adjustments"]>) {
    commitDocument((current) => ({
      ...current,
      adjustments: { ...current.adjustments, ...update },
    }));
  }

  const interactions = useCanvasInteractions({
    imageRef,
    canvasRef,
    scale: viewport.scale,
    tool,
    color,
    size,
    text,
    annotations,
    selection,
    setSelection,
    onSelect: (annotation) => {
      setSelectedAnnotationId(annotation?.id || null);
      if (annotation && "color" in annotation) {
        setColor(annotation.color);
        setSize(annotation.size);
        if (annotation.type === "text") setText(annotation.text);
      }
    },
    commitAnnotations,
  });
  const { draft } = interactions;

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
    let disposed = false;
    let unlisten: (() => void) | undefined;
    onCaptureLoaded(() => loadPendingCapture())
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch((error) => console.warn("Failed to subscribe to capture updates", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadPendingCapture]);

  useEffect(() => {
    return () => {
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
      setSelection({ x: 0, y: 0, width: image.naturalWidth, height: image.naturalHeight });
      resetDocument({ annotations: [], adjustments: DEFAULT_IMAGE_ADJUSTMENTS });
      interactions.resetInteraction();
      setSelectedAnnotationId(null);
      setTool("object");
      setStatus("Ready");
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
  }, [capture, resetDocument, updateViewport]);

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
    const renderedAnnotations = draft?.kind === "move"
      ? annotations.map((annotation) => annotation.id === draft.annotation.id ? draft.annotation : annotation)
      : annotations;
    const draftAnnotation = draft && draft.kind !== "crop" && draft.kind !== "move"
      ? draft.annotation
      : null;
    drawScene(
      canvas,
      image,
      viewport,
      selection,
      renderedAnnotations,
      draftAnnotation,
      adjustments,
      selectedAnnotationId,
    );
  }, [viewport, selection, annotations, draft, adjustments, capture, selectedAnnotationId]);

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

    renderExport(ctx, image, crop, annotations, adjustments);

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
    } catch (err) {
      console.error(err);
      setStatus("Recapture failed");
    } finally {
      setBusy(false);
    }
  }

  function resetEdits() {
    interactions.resetInteraction();
    setSelection(null);
    resetDocument({ annotations: [], adjustments: DEFAULT_IMAGE_ADJUSTMENTS });
    setSelectedAnnotationId(null);
    setTool("crop");
    setStatus("Select an area");
  }

  function updateSelectedStyle(update: { color?: string; size?: number; text?: string }) {
    if (!selectedAnnotationId) return;
    commitAnnotations((items) =>
      items.map((annotation) => {
        if (annotation.id !== selectedAnnotationId) return annotation;
        if (!("color" in annotation)) return annotation;
        return { ...annotation, ...update } as Annotation;
      }),
    );
  }

  function chooseColor(next: string) {
    setColor(next);
    updateSelectedStyle({ color: next });
  }

  function chooseSize(next: number) {
    setSize(next);
    updateSelectedStyle({ size: next });
  }

  function changeText(next: string) {
    setText(next);
    const selected = annotations.find((annotation) => annotation.id === selectedAnnotationId);
    if (selected?.type === "text") updateSelectedStyle({ text: next });
  }

  function deleteSelected() {
    if (!selectedAnnotationId) return;
    commitAnnotations((items) => items.filter((annotation) => annotation.id !== selectedAnnotationId));
    setSelectedAnnotationId(null);
  }

  function performUndo() {
    undo();
    setSelectedAnnotationId(null);
  }

  function performRedo() {
    redo();
    setSelectedAnnotationId(null);
  }

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
      const command = event.ctrlKey || event.metaKey;
      if (command && event.key.toLowerCase() === "z") {
        event.preventDefault();
        if (event.shiftKey) performRedo();
        else performUndo();
      } else if (command && event.key.toLowerCase() === "y") {
        event.preventDefault();
        performRedo();
      } else if (event.key === "Delete" || event.key === "Backspace") {
        event.preventDefault();
        deleteSelected();
      } else if (event.key === "Escape") {
        setSelectedAnnotationId(null);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [redo, selectedAnnotationId, undo]);

  return (
    <main className="capture-app">
      <EditorHeader busy={busy} onRecapture={() => void recapture()} onClose={() => void closeCurrentWindow()} />

      <section className="capture-body">
        <EditorSidebar
          tool={tool}
          color={color}
          size={size}
          text={text}
          adjustments={adjustments}
          hasSelection={selectedAnnotationId !== null}
          onTool={(nextTool) => {
            setTool(nextTool);
            if (nextTool !== "object") setSelectedAnnotationId(null);
          }}
          onColor={chooseColor}
          onSize={chooseSize}
          onText={changeText}
          onAdjust={commitAdjustments}
          onDelete={deleteSelected}
        />

        <section className="capture-stage-shell">
          <div className="capture-stage" ref={stageRef}>
            <canvas
              ref={canvasRef}
              className={`capture-canvas capture-canvas-${tool}`}
              style={{
                width: `${viewport.width}px`,
                height: `${viewport.height}px`,
                borderRadius: `${adjustments.cornerRadius * viewport.scale}px`,
              }}
              onPointerDown={interactions.onPointerDown}
              onPointerMove={interactions.onPointerMove}
              onPointerUp={interactions.onPointerUp}
              onPointerCancel={interactions.onPointerUp}
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

      <EditorFooter
        busy={busy}
        canUndo={canUndo}
        canRedo={canRedo}
        canExport={canExport}
        onUndo={performUndo}
        onRedo={performRedo}
        onReset={resetEdits}
        onExport={(action) => void runExport(action)}
      />
    </main>
  );
}
