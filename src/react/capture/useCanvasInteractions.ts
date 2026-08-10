import { useEffect, useRef, useState } from "react";
import { annotationAt, annotationBounds, translateAnnotation } from "./annotationGeometry";
import type { Annotation, Point, Rect, Tool } from "./types";

export type CanvasDragState =
  | { kind: "crop"; start: Point }
  | { kind: "pen"; annotation: Annotation }
  | { kind: "rect"; start: Point; annotation: Annotation }
  | { kind: "arrow"; start: Point; annotation: Annotation }
  | { kind: "effect"; start: Point; annotation: Annotation }
  | { kind: "move"; start: Point; initial: Annotation; annotation: Annotation }
  | null;

type Params = {
  imageRef: React.RefObject<HTMLImageElement | null>;
  canvasRef: React.RefObject<HTMLCanvasElement | null>;
  scale: number;
  tool: Tool;
  color: string;
  size: number;
  text: string;
  annotations: Annotation[];
  selection: Rect | null;
  setSelection: (selection: Rect | null) => void;
  onSelect: (annotation: Annotation | null) => void;
  commitAnnotations: (update: Annotation[] | ((items: Annotation[]) => Annotation[])) => void;
};

export function useCanvasInteractions(params: Params) {
  const draftRef = useRef<CanvasDragState>(null);
  const selectionRef = useRef<Rect | null>(params.selection);
  const pointerFrame = useRef<number | null>(null);
  const pendingSelection = useRef<Rect | null>(null);
  const pendingDraft = useRef<CanvasDragState>(null);
  const hasPendingSelection = useRef(false);
  const hasPendingDraft = useRef(false);
  const [draft, setDraft] = useState<CanvasDragState>(null);

  useEffect(() => {
    selectionRef.current = params.selection;
  }, [params.selection]);

  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  useEffect(() => () => {
    if (pointerFrame.current !== null) cancelAnimationFrame(pointerFrame.current);
  }, []);

  function flush() {
    if (hasPendingSelection.current) {
      hasPendingSelection.current = false;
      selectionRef.current = pendingSelection.current;
      params.setSelection(pendingSelection.current);
    }
    if (hasPendingDraft.current) {
      hasPendingDraft.current = false;
      const next = cloneDraft(pendingDraft.current);
      draftRef.current = next;
      setDraft(next);
    }
  }

  function schedule() {
    if (pointerFrame.current !== null) return;
    pointerFrame.current = requestAnimationFrame(() => {
      pointerFrame.current = null;
      flush();
    });
  }

  function setSelectionNow(next: Rect | null) {
    selectionRef.current = next;
    params.setSelection(next);
  }

  function setDraftNow(next: CanvasDragState) {
    draftRef.current = next;
    setDraft(next);
  }

  function scheduleSelection(next: Rect | null) {
    pendingSelection.current = next;
    hasPendingSelection.current = true;
    schedule();
  }

  function scheduleDraft(next: CanvasDragState) {
    pendingDraft.current = next;
    hasPendingDraft.current = true;
    schedule();
  }

  function cancelAndFlush() {
    if (pointerFrame.current !== null) {
      cancelAnimationFrame(pointerFrame.current);
      pointerFrame.current = null;
    }
    flush();
  }

  function pointFromEvent(event: React.PointerEvent<HTMLCanvasElement>): Point | null {
    const canvas = params.canvasRef.current;
    const image = params.imageRef.current;
    if (!canvas || !image || params.scale <= 0) return null;
    const bounds = canvas.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(image.naturalWidth, (event.clientX - bounds.left) / params.scale)),
      y: Math.max(0, Math.min(image.naturalHeight, (event.clientY - bounds.top) / params.scale)),
    };
  }

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    const point = pointFromEvent(event);
    if (!point) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    if (params.tool === "crop") {
      setDraftNow({ kind: "crop", start: point });
      setSelectionNow({ x: point.x, y: point.y, width: 0, height: 0 });
      params.onSelect(null);
    } else if (params.tool === "object") {
      const selected = annotationAt(params.annotations, point);
      params.onSelect(selected);
      setDraftNow(selected ? { kind: "move", start: point, initial: selected, annotation: selected } : null);
    } else if (params.tool === "pen") {
      setDraftNow({
        kind: "pen",
        annotation: { id: makeId("pen"), type: "pen", color: params.color, size: params.size, points: [point] },
      });
    } else if (params.tool === "rect") {
      setDraftNow({
        kind: "rect",
        start: point,
        annotation: {
          id: makeId("rect"), type: "rect", color: params.color, size: params.size,
          rect: { x: point.x, y: point.y, width: 0, height: 0 },
        },
      });
    } else if (params.tool === "arrow") {
      setDraftNow({
        kind: "arrow",
        start: point,
        annotation: {
          id: makeId("arrow"), type: "arrow", color: params.color, size: params.size, from: point, to: point,
        },
      });
    } else if (params.tool === "text" && params.text.trim()) {
      const annotation: Annotation = {
        id: makeId("text"), type: "text", color: params.color, size: params.size,
        at: point, text: params.text.trim(),
      };
      params.commitAnnotations((items) => [...items, annotation]);
      params.onSelect(annotation);
    } else if (params.tool === "blur" || params.tool === "mosaic") {
      setDraftNow({
        kind: "effect",
        start: point,
        annotation: {
          id: makeId(params.tool), type: params.tool,
          rect: { x: point.x, y: point.y, width: 0, height: 0 },
        },
      });
    }
  }

  function onPointerMove(event: React.PointerEvent<HTMLCanvasElement>) {
    const point = pointFromEvent(event);
    const active = draftRef.current;
    if (!point || !active) return;
    if (active.kind === "crop") {
      scheduleSelection(normalizeRect(active.start, point));
    } else if (active.kind === "pen" && active.annotation.type === "pen") {
      if (!shouldAppendPoint(active.annotation.points, point)) return;
      scheduleDraft({ ...active, annotation: { ...active.annotation, points: [...active.annotation.points, point] } });
    } else if (active.kind === "rect" && active.annotation.type === "rect") {
      scheduleDraft({ ...active, annotation: { ...active.annotation, rect: normalizeRect(active.start, point) } });
    } else if (active.kind === "arrow" && active.annotation.type === "arrow") {
      scheduleDraft({ ...active, annotation: { ...active.annotation, to: point } });
    } else if (active.kind === "effect" && (active.annotation.type === "blur" || active.annotation.type === "mosaic")) {
      scheduleDraft({ ...active, annotation: { ...active.annotation, rect: normalizeRect(active.start, point) } });
    } else if (active.kind === "move") {
      const image = params.imageRef.current;
      if (!image) return;
      const bounds = annotationBounds(active.initial);
      const delta = {
        x: Math.max(-bounds.x, Math.min(point.x - active.start.x, image.naturalWidth - bounds.x - bounds.width)),
        y: Math.max(-bounds.y, Math.min(point.y - active.start.y, image.naturalHeight - bounds.y - bounds.height)),
      };
      scheduleDraft({ ...active, annotation: translateAnnotation(active.initial, delta) });
    }
  }

  function onPointerUp() {
    cancelAndFlush();
    const active = draftRef.current;
    if (!active) return;
    if (active.kind === "crop") {
      const crop = selectionRef.current;
      setSelectionNow(crop && crop.width >= 3 && crop.height >= 3 ? crop : null);
    } else if (active.kind === "move") {
      params.commitAnnotations((items) =>
        items.map((annotation) => annotation.id === active.annotation.id ? active.annotation : annotation),
      );
    } else if (isValidDraft(active.annotation)) {
      params.commitAnnotations((items) => [...items, active.annotation]);
      params.onSelect(active.annotation);
    }
    setDraftNow(null);
  }

  function resetInteraction() {
    cancelAndFlush();
    setDraftNow(null);
  }

  return { draft, onPointerDown, onPointerMove, onPointerUp, resetInteraction };
}

function normalizeRect(start: Point, end: Point): Rect {
  return {
    x: Math.min(start.x, end.x),
    y: Math.min(start.y, end.y),
    width: Math.abs(start.x - end.x),
    height: Math.abs(start.y - end.y),
  };
}

function shouldAppendPoint(points: Point[], point: Point): boolean {
  const previous = points[points.length - 1];
  return !previous || (point.x - previous.x) ** 2 + (point.y - previous.y) ** 2 >= 0.36;
}

function cloneDraft(draft: CanvasDragState): CanvasDragState {
  if (!draft) return null;
  if (draft.kind === "crop") return { ...draft, start: { ...draft.start } };
  return { ...draft, annotation: cloneAnnotation(draft.annotation) } as CanvasDragState;
}

function cloneAnnotation(annotation: Annotation): Annotation {
  if (annotation.type === "pen") {
    return { ...annotation, points: annotation.points.map((point) => ({ ...point })) };
  }
  if (annotation.type === "arrow") {
    return { ...annotation, from: { ...annotation.from }, to: { ...annotation.to } };
  }
  if (annotation.type === "text") {
    return { ...annotation, at: { ...annotation.at } };
  }
  return { ...annotation, rect: { ...annotation.rect } };
}

function isValidDraft(annotation: Annotation): boolean {
  if (annotation.type === "pen") return annotation.points.length >= 2;
  if (annotation.type === "arrow") {
    return Math.hypot(annotation.to.x - annotation.from.x, annotation.to.y - annotation.from.y) >= 2;
  }
  if (annotation.type === "text") return annotation.text.length > 0;
  return annotation.rect.width >= 2 && annotation.rect.height >= 2;
}

function makeId(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}
