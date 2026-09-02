import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { drawScene } from "../annotation/canvasRenderer";
import { DEFAULT_IMAGE_ADJUSTMENTS, hasImageAdjustments, type ImageAdjustments } from "../annotation/imageAdjustments";
import { exportPngBase64, pngBase64ToObjectUrl } from "../annotation/pngPipeline";
import type { Annotation, Tool } from "../annotation/types";
import { useCanvasInteractions } from "../annotation/useCanvasInteractions";
import { useHistory } from "../annotation/useHistory";
import { DEFAULT_COLOR, DEFAULT_STROKE } from "../capture-overlay/tools";
import type { PinCanvasProject } from "./types";
import { parseInitialPinProject } from "./projectSchema";

function decodeImage(pngBase64: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = pngBase64ToObjectUrl(pngBase64);
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("Failed to decode pin source image"));
    };
    image.src = url;
  });
}

/** 贴图画布的唯一坐标系是 canonical source pixels；补偿预览从不进入此 hook。 */
export function usePinCanvas(params: {
  cssWidth: number;
  cssHeight: number;
  open: boolean;
  initialProject: unknown;
  loadSourceImage: () => Promise<string | null>;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sourceRef = useRef<HTMLImageElement | null>(null);
  const [sourceImage, setSourceImage] = useState<HTMLImageElement | null>(null);
  const parsedProject = useMemo(() => parseInitialPinProject(params.initialProject), [params.initialProject]);
  const history = useHistory<Annotation[]>([]);
  const annotations = history.value;
  const annotationsRef = useRef(annotations);
  annotationsRef.current = annotations;
  const [adjustments, setAdjustments] = useState<ImageAdjustments>({ ...DEFAULT_IMAGE_ADJUSTMENTS });
  const [tool, setTool] = useState<Tool>("pen");
  const [color, setColor] = useState(DEFAULT_COLOR);
  const [stroke, setStroke] = useState(DEFAULT_STROKE);
  const [text, setText] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [savedRevision, setSavedRevision] = useState(0);
  const hydratedRef = useRef<unknown>(null);

  useEffect(() => {
    if (!parsedProject || hydratedRef.current === params.initialProject) return;
    hydratedRef.current = params.initialProject;
    history.reset(parsedProject.annotations);
    setAdjustments(parsedProject.adjustments);
    setSelectedId(null);
    setRevision(0);
    setSavedRevision(0);
  }, [history.reset, params.initialProject, parsedProject]);

  const dirty = revision !== savedRevision;
  // 导入工程在发生第一次本地编辑前，IDAT 就是权威合成预览；无需让不同平台的 Canvas
  // 无意义地重放一次文字、滤镜和模糊。复制/再保存也可直接复用后端持有的预览。
  const pristineProject = parsedProject !== null && revision === 0;
  // 删除最后一个对象仍是一次未保存编辑；不能因此释放 source、复制旧预览或绕过工程保存。
  const hasDocument = parsedProject !== null
    || annotations.length > 0
    || hasImageAdjustments(adjustments)
    || dirty;
  const needsSource = params.open || (hasDocument && !pristineProject);
  const loadSource = useRef(params.loadSourceImage);
  loadSource.current = params.loadSourceImage;

  const releaseSource = useCallback(() => {
    const current = sourceRef.current;
    if (current) URL.revokeObjectURL(current.src);
    sourceRef.current = null;
    setSourceImage(null);
  }, []);

  useEffect(() => {
    if (!needsSource) {
      releaseSource();
      return;
    }
    if (sourceRef.current) return;
    let cancelled = false;
    void loadSource.current().then((base64) => {
      if (!base64) throw new Error("Pin source image is unavailable");
      return decodeImage(base64);
    }).then((image) => {
      if (cancelled) {
        URL.revokeObjectURL(image.src);
        return;
      }
      if (parsedProject && (image.naturalWidth !== parsedProject.sourceWidth || image.naturalHeight !== parsedProject.sourceHeight)) {
        URL.revokeObjectURL(image.src);
        throw new Error("Pin project source dimensions do not match");
      }
      sourceRef.current = image;
      setSourceImage(image);
    }).catch((reason) => console.error(reason));
    return () => {
      cancelled = true;
    };
  }, [needsSource, parsedProject, releaseSource]);

  useEffect(() => () => {
    const current = sourceRef.current;
    if (current) URL.revokeObjectURL(current.src);
  }, []);

  const sourceWidth = sourceImage?.naturalWidth ?? parsedProject?.sourceWidth ?? 0;
  const sourceHeight = sourceImage?.naturalHeight ?? parsedProject?.sourceHeight ?? 0;
  const scale = useMemo(() => (sourceWidth > 0 ? params.cssWidth / sourceWidth : 1), [params.cssWidth, sourceWidth]);

  const commitAnnotations = useCallback((update: Annotation[] | ((items: Annotation[]) => Annotation[])) => {
    history.commit(update);
    setRevision((value) => value + 1);
  }, [history.commit]);

  const imageRef = useRef<HTMLImageElement | null>(sourceImage);
  imageRef.current = sourceImage;
  const canvas = useCanvasInteractions({
    imageRef,
    canvasRef,
    scale,
    tool: tool === "crop" ? "crop" : tool,
    color,
    size: stroke,
    text,
    annotations,
    selection: null,
    setSelection: () => {},
    onSelect: (annotation) => setSelectedId(annotation?.id ?? null),
    commitAnnotations,
  });
  const draft = canvas.draft && "annotation" in canvas.draft ? canvas.draft.annotation : null;
  const visible = Boolean(sourceImage && (params.open || (hasDocument && !pristineProject)));

  useEffect(() => {
    const target = canvasRef.current;
    if (!visible || !target || !sourceImage) return;
    drawScene(
      target,
      sourceImage,
      { width: params.cssWidth, height: params.cssHeight, fitScale: scale, zoom: 1, scale },
      annotations,
      draft,
      adjustments,
      params.open ? selectedId : null,
    );
  }, [adjustments, annotations, draft, params.cssHeight, params.cssWidth, params.open, scale, selectedId, sourceImage, visible]);

  const exportPng = useCallback(async (): Promise<string> => {
    // 一次复制/保存必须锁定同一份文档快照；取原图或编码期间的新笔画属于下一 revision。
    const snapshotAnnotations = annotationsRef.current;
    let source = sourceRef.current;
    let temporary = false;
    if (!source) {
      const sourceBase64 = await loadSource.current();
      if (!sourceBase64) throw new Error("Pin source image is unavailable");
      source = await decodeImage(sourceBase64);
      temporary = true;
    }
    try {
      return await exportPngBase64(
        source,
        { x: 0, y: 0, width: source.naturalWidth, height: source.naturalHeight },
        snapshotAnnotations,
        adjustments,
      );
    } finally {
      if (temporary) URL.revokeObjectURL(source.src);
    }
  }, [adjustments]);

  const projectData = useMemo<PinCanvasProject | null>(() => sourceWidth > 0 && sourceHeight > 0 ? ({
    rendererVersion: 1,
    sourceWidth,
    sourceHeight,
    annotations,
    adjustments,
  }) : null, [adjustments, annotations, sourceHeight, sourceWidth]);
  const undo = useCallback(() => {
    if (!history.canUndo) return;
    history.undo();
    setRevision((value) => value + 1);
  }, [history.canUndo, history.undo]);
  const redo = useCallback(() => {
    if (!history.canRedo) return;
    history.redo();
    setRevision((value) => value + 1);
  }, [history.canRedo, history.redo]);
  const deleteSelected = useCallback(() => {
    if (!selectedId) return;
    commitAnnotations((items) => items.filter((item) => item.id !== selectedId));
    setSelectedId(null);
  }, [commitAnnotations, selectedId]);
  const markSaved = useCallback(() => setSavedRevision(revision), [revision]);

  return useMemo(() => ({
    canvasRef,
    visible,
    tool,
    setTool,
    color,
    setColor,
    stroke,
    setStroke,
    text,
    setText,
    hasSelectedObject: selectedId !== null,
    canUndo: history.canUndo,
    canRedo: history.canRedo,
    undo,
    redo,
    dirty,
    hasDocument,
    pristineProject,
    exportPng,
    projectData,
    markSaved,
    deleteSelected,
    onPointerDown: canvas.onPointerDown,
    onPointerMove: canvas.onPointerMove,
    onPointerUp: canvas.onPointerUp,
  }), [
    canvas.onPointerDown, canvas.onPointerMove, canvas.onPointerUp, color, deleteSelected,
    exportPng, hasDocument, history.canRedo, history.canUndo, markSaved, projectData, redo,
    dirty, pristineProject, selectedId, stroke, text, tool, undo, visible,
  ]);
}
