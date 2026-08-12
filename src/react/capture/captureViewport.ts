import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type { RenderViewport } from "./canvasRenderer";

export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 6;

const STAGE_PADDING = 24;
const MIN_STAGE_WIDTH = 320;
const MIN_STAGE_HEIGHT = 240;

export const INITIAL_VIEWPORT: RenderViewport = {
  width: 1,
  height: 1,
  fitScale: 1,
  zoom: 1,
  scale: 1,
};

type StageSize = Pick<HTMLElement, "clientWidth" | "clientHeight">;
type ImageSize = Pick<HTMLImageElement, "naturalWidth" | "naturalHeight">;

export function clampZoom(value: number): number {
  return Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, value));
}

export function zoomFromWheel(baseZoom: number, deltaY: number): number {
  return clampZoom(baseZoom * Math.exp(-deltaY * 0.002));
}

export function buildViewport(stage: StageSize, image: ImageSize, zoom: number): RenderViewport {
  const maxWidth = Math.max(MIN_STAGE_WIDTH, stage.clientWidth - STAGE_PADDING);
  const maxHeight = Math.max(MIN_STAGE_HEIGHT, stage.clientHeight - STAGE_PADDING);
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

export function useCaptureViewport(
  stageRef: RefObject<HTMLDivElement | null>,
  imageRef: RefObject<HTMLImageElement | null>,
) {
  const zoomRef = useRef(1);
  const pendingZoomRef = useRef<number | null>(null);
  const wheelFrameRef = useRef<number | null>(null);
  const [viewport, setViewport] = useState<RenderViewport>(INITIAL_VIEWPORT);

  useEffect(() => {
    zoomRef.current = viewport.zoom;
  }, [viewport.zoom]);

  const updateViewport = useCallback((zoomOverride?: number) => {
    const stage = stageRef.current;
    const image = imageRef.current;
    if (!stage || !image) return;
    setViewport((current) => buildViewport(stage, image, zoomOverride ?? current.zoom));
  }, [imageRef, stageRef]);

  useEffect(() => {
    updateViewport();
    const stage = stageRef.current;
    if (!stage || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => updateViewport());
    observer.observe(stage);
    return () => observer.disconnect();
  }, [stageRef, updateViewport]);

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;

    function onWheel(event: WheelEvent) {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      const baseZoom = pendingZoomRef.current ?? zoomRef.current;
      const nextZoom = zoomFromWheel(baseZoom, event.deltaY);
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
    return () => {
      stage.removeEventListener("wheel", onWheel);
      if (wheelFrameRef.current !== null) {
        cancelAnimationFrame(wheelFrameRef.current);
        wheelFrameRef.current = null;
      }
    };
  }, [stageRef, updateViewport]);

  return { viewport, updateViewport };
}
