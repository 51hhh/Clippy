import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { PointerEvent, KeyboardEvent } from "react";
import type { Annotation, Tool } from "../annotation/types";
import type { ImageAdjustments } from "../annotation/imageAdjustments";
import { drawScene } from "../annotation/canvasRenderer";
import { useCanvasInteractions } from "../annotation/useCanvasInteractions";
import { boundedDpr, clamp, fitView, imageOrigin, sourcePoint, zoomAt, type Size, type View } from "./geometry";

type Props = {
  image: HTMLImageElement | null; source: Size; blocked: boolean; tool: Tool | "pan" | "color";
  color: string; stroke: number; text: string; annotations: Annotation[]; adjustments: ImageAdjustments;
  selectedId: string | null; select: (id: string | null) => void;
  commit: (update: Annotation[] | ((items: Annotation[]) => Annotation[])) => void;
  sample: (x: number, y: number) => void; onError: () => void;
};

export function useViewerCanvas(props: Props) {
  const stageRef = useRef<HTMLDivElement>(null), canvasRef = useRef<HTMLCanvasElement>(null);
  const imageRef = useRef(props.image); imageRef.current = props.image;
  const [size, setSize] = useState({ width: 1, height: 1 });
  const [dpr, setDpr] = useState(window.devicePixelRatio || 1);
  const [view, setView] = useState<View>({ scale: 1, x: 0, y: 0, mode: "fit" });
  const [panning, setPanning] = useState(false);
  const space = useRef(false), pointer = useRef<{ id: number; x: number; y: number; start: View; pan: boolean } | null>(null);
  const latest = useRef({ props, size, view }); latest.current = { props, size, view };
  const interactions = useCanvasInteractions({ imageRef, canvasRef, scale: view.scale,
    pointFromClient: (clientX, clientY) => {
      const bounds = canvasRef.current?.getBoundingClientRect();
      return bounds ? sourcePoint({ x: clientX - bounds.left, y: clientY - bounds.top }, view, size, props.source, true) : null;
    }, tool: props.tool === "pan" || props.tool === "color" ? "object" : props.tool,
    color: props.color, size: props.stroke, text: props.text, annotations: props.annotations,
    selection: null, setSelection() {}, onSelect: annotation => props.select(annotation?.id || null), commitAnnotations: props.commit,
  });
  const interactionRef = useRef(interactions); interactionRef.current = interactions;
  function cancel() {
    const active = pointer.current; pointer.current = null; space.current = false; setPanning(false);
    interactionRef.current.resetInteraction();
    if (active && canvasRef.current?.hasPointerCapture?.(active.id)) canvasRef.current.releasePointerCapture(active.id);
  }
  useLayoutEffect(() => {
    const measure = () => {
      const rect = stageRef.current!.getBoundingClientRect();
      setSize(previous => {
        const next = { width: Math.max(1, Math.min(8192, rect.width)), height: Math.max(1, Math.min(8192, rect.height)) };
        return next.width === previous.width && next.height === previous.height ? previous : next;
      });
      setDpr(window.devicePixelRatio || 1);
    };
    measure(); const observer = new ResizeObserver(measure); observer.observe(stageRef.current!);
    window.addEventListener("resize", measure);
    return () => { observer.disconnect(); window.removeEventListener("resize", measure); };
  }, []);
  useEffect(() => {
    // 换到不同 DPI 的屏幕时 resolution media query 会失效，重新绑定新 DPR。
    if (!window.matchMedia) return;
    const media = window.matchMedia(`(resolution: ${dpr}dppx)`);
    const changed = () => setDpr(window.devicePixelRatio || 1);
    media.addEventListener("change", changed);
    return () => media.removeEventListener("change", changed);
  }, [dpr]);
  useLayoutEffect(() => {
    cancel();
    setView(previous => previous.mode === "fit" ? fitView(size, props.source)
      : { ...previous, scale: previous.mode === "actual" ? 1 / dpr : previous.scale });
  }, [size.width, size.height, dpr, props.source.width, props.source.height]);
  useEffect(() => {
    cancel();
  }, [props.blocked, props.tool]);
  useEffect(() => {
    const hidden = () => { if (document.hidden) cancel(); };
    window.addEventListener("blur", cancel); document.addEventListener("visibilitychange", hidden);
    return () => { window.removeEventListener("blur", cancel); document.removeEventListener("visibilitychange", hidden); };
  }, []);
  useEffect(() => {
    const canvas = canvasRef.current!;
    const wheel = (event: WheelEvent) => {
      if (latest.current.props.blocked || !latest.current.props.image) return;
      event.preventDefault(); cancel();
      const rect = canvas.getBoundingClientRect();
      const { size: area, props: values } = latest.current;
      setView(previous => zoomAt(previous, previous.scale * Math.exp(-clamp(event.deltaY, -400, 400) * .002),
        { x: event.clientX - rect.left, y: event.clientY - rect.top }, area, values.source));
    };
    canvas.addEventListener("wheel", wheel, { passive: false });
    return () => canvas.removeEventListener("wheel", wheel);
  }, []);
  const draft = interactions.draft && "annotation" in interactions.draft ? interactions.draft.annotation : null;
  useEffect(() => {
    if (!props.image || !canvasRef.current) return;
    try {
      drawScene(canvasRef.current, props.image,
        { ...size, scale: view.scale, fitScale: view.scale, zoom: 1, pixelRatio: boundedDpr(dpr, size), origin: imageOrigin(view, size, props.source) },
        props.annotations, draft, props.adjustments, props.selectedId);
    } catch { props.onError(); }
  }, [props.image, props.annotations, props.adjustments, props.selectedId, size, view, draft, dpr]);
  function down(event: PointerEvent<HTMLCanvasElement>) {
    if (props.blocked || !props.image || (event.button !== 0 && event.button !== 1)) return;
    event.preventDefault(); event.currentTarget.focus({ preventScroll: true });
    const rect = event.currentTarget.getBoundingClientRect();
    const point = sourcePoint({ x: event.clientX - rect.left, y: event.clientY - rect.top }, view, size, props.source);
    const pan = props.tool === "pan" || space.current || event.button === 1;
    if (!pan && !point) return;
    if (props.tool === "color" && !pan && point) { props.sample(Math.floor(point.x), Math.floor(point.y)); return; }
    pointer.current = { id: event.pointerId, x: event.clientX, y: event.clientY, start: view, pan };
    if (pan) { event.currentTarget.setPointerCapture(event.pointerId); setPanning(true); }
    else interactions.onPointerDown(event);
  }
  function move(event: PointerEvent<HTMLCanvasElement>) {
    const active = pointer.current;
    if (props.blocked || !active || active.id !== event.pointerId) return;
    if (active.pan) setView({ ...active.start, x: active.start.x + event.clientX - active.x, y: active.start.y + event.clientY - active.y, mode: "custom" });
    else interactions.onPointerMove(event);
  }
  function up(event: PointerEvent<HTMLCanvasElement>) {
    const active = pointer.current;
    if (!active || active.id !== event.pointerId) return;
    pointer.current = null; setPanning(false);
    if (!props.blocked && !active.pan) interactions.onPointerUp(); else interactions.resetInteraction();
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  }
  const fit = () => { cancel(); setView(fitView(size, props.source)); };
  const actual = () => { cancel(); setView({ scale: 1 / dpr, x: 0, y: 0, mode: "actual" }); };
  const zoom = (factor: number) => { cancel(); setView(previous => zoomAt(previous, previous.scale * factor, { x: size.width / 2, y: size.height / 2 }, size, props.source)); };
  function key(event: KeyboardEvent<HTMLCanvasElement>) {
    if (props.blocked || event.ctrlKey || event.metaKey || event.altKey || event.nativeEvent.isComposing) return;
    if (event.key === " ") { event.preventDefault(); space.current = true; }
    else if (["+", "=", "-", "0", "1", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) {
      event.preventDefault();
      if (event.key === "0") fit(); else if (event.key === "1") actual();
      else if (["+", "=", "-"].includes(event.key)) zoom(event.key === "-" ? .8 : 1.25);
      else { cancel(); setView(previous => ({ ...previous, mode: "custom", x: previous.x + (event.key === "ArrowLeft" ? 48 : event.key === "ArrowRight" ? -48 : 0), y: previous.y + (event.key === "ArrowUp" ? 48 : event.key === "ArrowDown" ? -48 : 0) })); }
    }
  }
  return { stageRef, canvasRef, size, view, dpr, panning, fit, actual, zoom, cancel,
    handlers: { onPointerDown: down, onPointerMove: move, onPointerUp: up, onPointerCancel: cancel,
      onLostPointerCapture: () => { if (pointer.current) cancel(); }, onBlur: cancel, onKeyDown: key,
      onKeyUp: (event: KeyboardEvent) => { if (event.key === " ") space.current = false; } } };
}
