import { annotationBounds } from "./annotationGeometry";
import { cssFilterForImageAdjustments, type ImageAdjustments } from "./imageAdjustments";
import type { Annotation, Point, Rect } from "./types";

export type RenderViewport = {
  width: number;
  height: number;
  fitScale: number;
  zoom: number;
  scale: number;
};

const MAX_CANVAS_DPR = 2;

export function drawScene(
  canvas: HTMLCanvasElement,
  image: HTMLImageElement,
  viewport: RenderViewport,
  crop: Rect | null,
  annotations: Annotation[],
  draft: Annotation | null,
  adjustments: ImageAdjustments,
  selectedId: string | null,
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
  drawBaseImage(ctx, image, adjustments, viewport.scale, { x: 0, y: 0 });
  drawEffects(ctx, image, annotations, adjustments, viewport.scale, { x: 0, y: 0 });
  annotations.filter(isVectorAnnotation).forEach((annotation) => drawAnnotation(ctx, annotation, viewport.scale));
  if (draft) {
    if (isVectorAnnotation(draft)) drawAnnotation(ctx, draft, viewport.scale);
    else drawEffect(ctx, image, draft, adjustments, viewport.scale, { x: 0, y: 0 });
  }
  if (selectedId) {
    const selected = annotations.find((annotation) => annotation.id === selectedId);
    if (selected) drawSelectedBounds(ctx, annotationBounds(selected), viewport.scale);
  }
  if (crop) drawCropOverlay(ctx, crop, viewport);
}

export function renderExport(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  crop: Rect,
  annotations: Annotation[],
  adjustments: ImageAdjustments,
) {
  ctx.save();
  ctx.filter = cssFilterForImageAdjustments(adjustments);
  ctx.drawImage(image, crop.x, crop.y, crop.width, crop.height, 0, 0, crop.width, crop.height);
  ctx.restore();
  const offset = { x: -crop.x, y: -crop.y };
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, crop.width, crop.height);
  ctx.clip();
  drawEffects(ctx, image, annotations, adjustments, 1, offset);
  annotations.filter(isVectorAnnotation).forEach((annotation) => drawAnnotation(ctx, annotation, 1, offset));
  ctx.restore();
  if (adjustments.cornerRadius > 0) applyRoundedMask(ctx, crop.width, crop.height, adjustments.cornerRadius);
}

function drawBaseImage(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  adjustments: ImageAdjustments,
  scale: number,
  offset: Point,
) {
  ctx.save();
  ctx.filter = cssFilterForImageAdjustments(adjustments);
  ctx.drawImage(
    image,
    offset.x * scale,
    offset.y * scale,
    image.naturalWidth * scale,
    image.naturalHeight * scale,
  );
  ctx.restore();
}

function drawEffects(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  annotations: Annotation[],
  adjustments: ImageAdjustments,
  scale: number,
  offset: Point,
) {
  annotations.filter(isEffectAnnotation).forEach((annotation) =>
    drawEffect(ctx, image, annotation, adjustments, scale, offset),
  );
}

function drawEffect(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  annotation: Extract<Annotation, { type: "blur" | "mosaic" }>,
  adjustments: ImageAdjustments,
  scale: number,
  offset: Point,
) {
  const rect = annotation.rect;
  const destination = {
    x: (rect.x + offset.x) * scale,
    y: (rect.y + offset.y) * scale,
    width: rect.width * scale,
    height: rect.height * scale,
  };
  if (destination.width <= 0 || destination.height <= 0) return;

  ctx.save();
  ctx.beginPath();
  ctx.rect(destination.x, destination.y, destination.width, destination.height);
  ctx.clip();
  if (annotation.type === "blur") {
    const filter = cssFilterForImageAdjustments(adjustments);
    ctx.filter = `${filter === "none" ? "" : `${filter} `}blur(${Math.max(4, 8 * scale)}px)`;
    ctx.drawImage(
      image,
      offset.x * scale,
      offset.y * scale,
      image.naturalWidth * scale,
      image.naturalHeight * scale,
    );
  } else {
    const cell = Math.max(6, 12 / Math.max(scale, 0.01));
    const width = Math.max(1, Math.ceil(rect.width / cell));
    const height = Math.max(1, Math.ceil(rect.height / cell));
    const buffer = document.createElement("canvas");
    buffer.width = width;
    buffer.height = height;
    const source = buffer.getContext("2d");
    if (source) {
      source.filter = cssFilterForImageAdjustments(adjustments);
      source.drawImage(image, rect.x, rect.y, rect.width, rect.height, 0, 0, width, height);
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(buffer, destination.x, destination.y, destination.width, destination.height);
      ctx.imageSmoothingEnabled = true;
    }
  }
  ctx.restore();
}

export function drawAnnotation(
  ctx: CanvasRenderingContext2D,
  annotation: Exclude<Annotation, { type: "blur" | "mosaic" }>,
  scale = 1,
  offset: Point = { x: 0, y: 0 },
) {
  ctx.save();
  ctx.strokeStyle = annotation.color;
  ctx.fillStyle = annotation.color;
  ctx.lineWidth = Math.max(1, annotation.size * scale);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  const point = (value: Point) => ({
    x: (value.x + offset.x) * scale,
    y: (value.y + offset.y) * scale,
  });

  if (annotation.type === "pen") {
    if (annotation.points.length >= 2) {
      ctx.beginPath();
      const first = point(annotation.points[0]);
      ctx.moveTo(first.x, first.y);
      annotation.points.slice(1).forEach((item) => {
        const next = point(item);
        ctx.lineTo(next.x, next.y);
      });
      ctx.stroke();
    }
  } else if (annotation.type === "rect") {
    ctx.strokeRect(
      (annotation.rect.x + offset.x) * scale,
      (annotation.rect.y + offset.y) * scale,
      annotation.rect.width * scale,
      annotation.rect.height * scale,
    );
  } else if (annotation.type === "arrow") {
    const from = point(annotation.from);
    const to = point(annotation.to);
    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.stroke();
    drawArrowHead(ctx, from, to, annotation.size * scale);
  } else {
    const at = point(annotation.at);
    const fontSize = Math.max(14, annotation.size * 4) * scale;
    ctx.font = `600 ${fontSize}px system-ui, sans-serif`;
    ctx.textBaseline = "top";
    ctx.lineWidth = Math.max(3, annotation.size * scale);
    ctx.strokeStyle = "rgba(0, 0, 0, 0.55)";
    ctx.strokeText(annotation.text, at.x, at.y);
    ctx.fillStyle = annotation.color;
    ctx.fillText(annotation.text, at.x, at.y);
  }
  ctx.restore();
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

function drawSelectedBounds(ctx: CanvasRenderingContext2D, rect: Rect, scale: number) {
  ctx.save();
  ctx.strokeStyle = "#ffffff";
  ctx.lineWidth = 1;
  ctx.setLineDash([5, 4]);
  ctx.strokeRect(rect.x * scale - 3, rect.y * scale - 3, rect.width * scale + 6, rect.height * scale + 6);
  ctx.restore();
}

function drawCropOverlay(ctx: CanvasRenderingContext2D, crop: Rect, viewport: RenderViewport) {
  const x = crop.x * viewport.scale;
  const y = crop.y * viewport.scale;
  const width = crop.width * viewport.scale;
  const height = crop.height * viewport.scale;
  ctx.save();
  ctx.fillStyle = "rgba(0, 0, 0, 0.34)";
  ctx.beginPath();
  ctx.rect(0, 0, viewport.width, viewport.height);
  ctx.rect(x, y, width, height);
  ctx.fill("evenodd");
  ctx.strokeStyle = "#0a84ff";
  ctx.lineWidth = 2;
  ctx.setLineDash([8, 5]);
  ctx.strokeRect(x, y, width, height);
  ctx.restore();
}

function applyRoundedMask(ctx: CanvasRenderingContext2D, width: number, height: number, radius: number) {
  const safeRadius = Math.max(0, Math.min(radius, width / 2, height / 2));
  ctx.save();
  ctx.globalCompositeOperation = "destination-in";
  ctx.beginPath();
  ctx.roundRect(0, 0, width, height, safeRadius);
  ctx.fill();
  ctx.restore();
}

function isEffectAnnotation(
  annotation: Annotation,
): annotation is Extract<Annotation, { type: "blur" | "mosaic" }> {
  return annotation.type === "blur" || annotation.type === "mosaic";
}

function isVectorAnnotation(
  annotation: Annotation,
): annotation is Exclude<Annotation, { type: "blur" | "mosaic" }> {
  return !isEffectAnnotation(annotation);
}
