import { useEffect, useMemo, useRef, useState } from "react";
import {
  closeCurrentWindow,
  copyScreenshotImage,
  pinScreenshotImage,
  saveScreenshotImage,
  saveScreenshotImageAs,
  showCaptureEditor,
} from "../../js/api.ts";
import { drawScene } from "./canvasRenderer";
import { useCaptureViewport } from "./captureViewport";
import { EditorFooter, EditorHeader, type ExportAction } from "./EditorChrome";
import { EditorSidebar } from "./EditorSidebar";
import { DEFAULT_IMAGE_ADJUSTMENTS } from "./imageAdjustments";
import { exportPngBase64, isExportSelection } from "./pngPipeline";
import { useCanvasInteractions } from "./useCanvasInteractions";
import { useHistory } from "./useHistory";
import { usePendingCaptureImage } from "./usePendingCaptureImage";
import type { Annotation, EditorDocument, Rect, Tool } from "./types";
import { t } from "../shared/i18n";

const DEFAULT_COLOR = "#ff3b30";

export function App() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const { viewport, updateViewport } = useCaptureViewport(stageRef, imageRef);
  const [selection, setSelection] = useState<Rect | null>(null);
  const [tool, setTool] = useState<Tool>("object");
  const [selectedAnnotationId, setSelectedAnnotationId] = useState<string | null>(null);
  const [color, setColor] = useState(DEFAULT_COLOR);
  const [size, setSize] = useState(4);
  const [text, setText] = useState(() => t("capture.defaultText"));
  const [status, setStatus] = useState(() => t("capture.loading"));
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

  const { imageReady, pendingCapture, releaseImage } = usePendingCaptureImage({
    canvasRef,
    imageRef,
    onImageReady: (image) => {
      setSelection({ x: 0, y: 0, width: image.naturalWidth, height: image.naturalHeight });
      resetDocument({ annotations: [], adjustments: DEFAULT_IMAGE_ADJUSTMENTS });
      interactions.resetInteraction();
      setSelectedAnnotationId(null);
      setTool("object");
      setStatus(t("capture.ready"));
      updateViewport(1);
    },
    onStatus: setStatus,
  });

  const activeImage = imageReady ? imageRef.current : null;
  const exportRect = useMemo(() => {
    if (!activeImage) return null;
    return isExportSelection(selection) ? selection : null;
  }, [activeImage, imageReady, selection]);
  const canExport = !!activeImage && !!exportRect && !busy;

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
  }, [viewport, selection, annotations, draft, adjustments, pendingCapture, selectedAnnotationId]);

  async function runExport(action: ExportAction) {
    if (!canExport) return;
    setBusy(true);
    setStatus(t(`capture.${action}Progress`));
    try {
      await new Promise((resolve) => requestAnimationFrame(resolve));
      const image = imageRef.current;
      if (!image) throw new Error("No screenshot loaded");
      if (!exportRect) throw new Error("Select an area first");
      const pngBase64 = await exportPngBase64(image, exportRect, annotations, adjustments);
      if (action === "copy") {
        await copyScreenshotImage(pngBase64);
        setStatus(t("capture.copied"));
      } else if (action === "save") {
        const path = await saveScreenshotImage(pngBase64);
        setStatus(t("capture.saved", { path }));
      } else if (action === "saveAs") {
        // 用户在对话框里取消时后端返回 null，这不是失败。
        const path = await saveScreenshotImageAs(pngBase64);
        setStatus(path ? t("capture.saved", { path }) : t("capture.saveAsCancelled"));
      } else {
        await pinScreenshotImage(pngBase64);
        setStatus(t("capture.pinned"));
      }
    } catch (err) {
      console.error(err);
      setStatus(t(`capture.${action}Failed`));
    } finally {
      setBusy(false);
    }
  }

  async function recapture() {
    setBusy(true);
    setStatus(t("capture.recapturing"));
    try {
      await showCaptureEditor();
    } catch (err) {
      console.error(err);
      setStatus(t("capture.recaptureFailed"));
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
    setStatus(t("capture.selectArea"));
  }

  function closeEditor() {
    releaseImage();
    void closeCurrentWindow();
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
      <EditorHeader busy={busy} onRecapture={() => void recapture()} onClose={closeEditor} />

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
                  ? t("capture.noSelection")
                  : t("capture.noImage")}
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
