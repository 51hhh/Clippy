import { annotationBounds, isEffectAnnotation, isVectorAnnotation } from "./annotationGeometry";
import { type FrameImage, frameHeight, frameWidth } from "./frameImage";
import { cssFilterForImageAdjustments, type ImageAdjustments } from "./imageAdjustments";
import {
  DEFAULT_EFFECT_PARAMETERS,
  type Annotation,
  type EffectAnnotation,
  type Point,
  type Rect,
  type SegmentAnnotation,
  type VectorAnnotation,
} from "./types";

/**
 * 这里的 Canvas 2D 只负责交互预览和像素 smoke，不是最终渲染事实源。
 * Pin 工程与截图覆盖层的 Copy/Save/Pin 都会把 v2 操作层交给 Rust 固定字体与
 * 软件光栅器。旧 v1 工程未修改时复用 IDAT，第一次真实编辑后再升级。
 */

export type RenderViewport = {
  width: number;
  height: number;
  fitScale: number;
  zoom: number;
  scale: number;
  /** CSS 像素到画布物理像素的倍率；未指定时沿用窗口 DPR。 */
  pixelRatio?: number;
};

const MAX_CANVAS_DPR = 2;
/** 半透明注解（高亮矩形、荧光笔）的不透明度 */
const HIGHLIGHT_ALPHA = 0.32;
/** 荧光笔笔尖相对线宽的倍数 */
const MARKER_WIDTH_FACTOR = 2.6;

/**
 * 把底图、效果类标注、矢量标注和"选中标注"的虚线框画成一帧。
 *
 * **裁剪选区的压暗与虚线蓝框不在这里**，它们是 DOM 的事（覆盖层的 `.selection`
 * 用 `outline` + `box-shadow` 画）。原因是性能：这个函数每次都要把整张冻结帧
 * （本机 2560×1600）以 `imageSmoothingQuality = "high"` 缩绘进 1920×1200 的画布，
 * 而拖动/缩放选区时每一个 pointermove 都会改变选区矩形。压暗留在画布上意味着
 * 纯粹为了重画一层半透明蒙版和一个虚线框，每帧白做一次全图重采样；交给合成器之后
 * 拖选区引发的画布重绘次数是 0。导出路径 `renderExport` 本来就不画压暗（那是取景
 * 辅助线，不是画面内容），所以这一层职责搬走不影响产物。
 */
export function drawScene(
  canvas: HTMLCanvasElement,
  image: FrameImage,
  viewport: RenderViewport,
  annotations: Annotation[],
  draft: Annotation | null,
  adjustments: ImageAdjustments,
  selectedId: string | null,
) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const dpr = viewport.pixelRatio === undefined
    ? Math.min(MAX_CANVAS_DPR, Math.max(1, window.devicePixelRatio || 1))
    : Math.max(1, viewport.pixelRatio);
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
}

export function renderExport(
  ctx: CanvasRenderingContext2D,
  image: FrameImage,
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
  image: FrameImage,
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
    frameWidth(image) * scale,
    frameHeight(image) * scale,
  );
  ctx.restore();
}

function drawEffects(
  ctx: CanvasRenderingContext2D,
  image: FrameImage,
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
  image: FrameImage,
  annotation: EffectAnnotation,
  adjustments: ImageAdjustments,
  scale: number,
  offset: Point,
) {
  const rect = annotation.rect;
  const effect = annotation.effect ?? DEFAULT_EFFECT_PARAMETERS;
  const destination = {
    x: (rect.x + offset.x) * scale,
    y: (rect.y + offset.y) * scale,
    width: rect.width * scale,
    height: rect.height * scale,
  };
  if (destination.width <= 0 || destination.height <= 0) return;

  if (annotation.type === "spotlight") {
    drawSpotlight(ctx, image, destination, scale, offset, effect.spotlightDim);
    return;
  }
  if (annotation.type === "magnifier") {
    drawMagnifier(ctx, image, rect, destination, adjustments, scale, effect.magnifierZoom);
    return;
  }

  ctx.save();
  ctx.beginPath();
  ctx.rect(destination.x, destination.y, destination.width, destination.height);
  ctx.clip();
  if (annotation.type === "blur") {
    const filter = cssFilterForImageAdjustments(adjustments);
    ctx.filter = `${filter === "none" ? "" : `${filter} `}blur(${Math.max(4, effect.blurRadius * scale)}px)`;
    ctx.drawImage(
      image,
      offset.x * scale,
      offset.y * scale,
      frameWidth(image) * scale,
      frameHeight(image) * scale,
    );
  } else {
    const cell = Math.max(6, effect.mosaicCell / Math.max(scale, 0.01));
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

/** 聚光灯：把选区之外的底图压暗，选区内保持原样 */
function drawSpotlight(
  ctx: CanvasRenderingContext2D,
  image: FrameImage,
  destination: Rect,
  scale: number,
  offset: Point,
  dim: number,
) {
  ctx.save();
  ctx.beginPath();
  ctx.rect(offset.x * scale, offset.y * scale, frameWidth(image) * scale, frameHeight(image) * scale);
  ctx.rect(destination.x, destination.y, destination.width, destination.height);
  ctx.fillStyle = `rgba(0, 0, 0, ${dim})`;
  ctx.fill("evenodd");
  ctx.restore();
}

/**
 * 放大镜：在选区内画同一块底图的放大版本，中心对齐选区中心。
 * 采样用的是原图而不是已经缩小的画布，因此预览和导出的清晰度一致。
 */
function drawMagnifier(
  ctx: CanvasRenderingContext2D,
  image: FrameImage,
  rect: Rect,
  destination: Rect,
  adjustments: ImageAdjustments,
  scale: number,
  zoom: number,
) {
  const centerX = destination.x + destination.width / 2;
  const centerY = destination.y + destination.height / 2;
  const zoomed = scale * zoom;
  ctx.save();
  ctx.beginPath();
  ctx.ellipse(centerX, centerY, destination.width / 2, destination.height / 2, 0, 0, Math.PI * 2);
  ctx.clip();
  ctx.filter = cssFilterForImageAdjustments(adjustments);
  ctx.drawImage(
    image,
    centerX - (rect.x + rect.width / 2) * zoomed,
    centerY - (rect.y + rect.height / 2) * zoomed,
    frameWidth(image) * zoomed,
    frameHeight(image) * zoomed,
  );
  ctx.restore();

  ctx.save();
  ctx.strokeStyle = "rgba(255, 255, 255, 0.92)";
  ctx.lineWidth = Math.max(1, 2 * scale);
  ctx.beginPath();
  ctx.ellipse(centerX, centerY, destination.width / 2, destination.height / 2, 0, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

export function drawAnnotation(
  ctx: CanvasRenderingContext2D,
  annotation: VectorAnnotation,
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

  switch (annotation.type) {
    case "pen":
    case "marker": {
      if (annotation.points.length < 2) break;
      if (annotation.type === "marker") {
        // 荧光笔：更粗且半透明，用一条连续路径描完，重叠处不会越描越黑。
        ctx.globalAlpha = HIGHLIGHT_ALPHA;
        ctx.lineWidth = Math.max(2, annotation.size * MARKER_WIDTH_FACTOR * scale);
        ctx.lineCap = "butt";
      }
      ctx.beginPath();
      const first = point(annotation.points[0]);
      ctx.moveTo(first.x, first.y);
      annotation.points.slice(1).forEach((item) => {
        const next = point(item);
        ctx.lineTo(next.x, next.y);
      });
      ctx.stroke();
      break;
    }
    case "rect": {
      const rect = annotation.rect;
      ctx.strokeRect((rect.x + offset.x) * scale, (rect.y + offset.y) * scale, rect.width * scale, rect.height * scale);
      break;
    }
    case "highlight": {
      const rect = annotation.rect;
      ctx.globalAlpha = HIGHLIGHT_ALPHA;
      ctx.fillRect((rect.x + offset.x) * scale, (rect.y + offset.y) * scale, rect.width * scale, rect.height * scale);
      break;
    }
    case "ellipse": {
      const rect = annotation.rect;
      ctx.beginPath();
      ctx.ellipse(
        (rect.x + rect.width / 2 + offset.x) * scale,
        (rect.y + rect.height / 2 + offset.y) * scale,
        Math.max(0, (rect.width / 2) * scale),
        Math.max(0, (rect.height / 2) * scale),
        0,
        0,
        Math.PI * 2,
      );
      ctx.stroke();
      break;
    }
    case "arrow":
    case "line":
    case "measure": {
      const from = point(annotation.from);
      const to = point(annotation.to);
      ctx.beginPath();
      ctx.moveTo(from.x, from.y);
      ctx.lineTo(to.x, to.y);
      ctx.stroke();
      if (annotation.type === "arrow") drawArrowHead(ctx, from, to, annotation.size * scale);
      if (annotation.type === "measure") drawMeasureDecoration(ctx, annotation, from, to, scale);
      break;
    }
    case "text": {
      const at = point(annotation.at);
      const fontSize = Math.max(14, annotation.size * 4) * scale;
      ctx.font = `600 ${fontSize}px ${annotation.fontFamily ?? "system-ui"}, sans-serif`;
      ctx.textBaseline = "top";
      ctx.lineWidth = Math.max(3, annotation.size * scale);
      ctx.strokeStyle = "rgba(0, 0, 0, 0.55)";
      ctx.strokeText(annotation.text, at.x, at.y);
      ctx.fillStyle = annotation.color;
      ctx.fillText(annotation.text, at.x, at.y);
      break;
    }
  }
  ctx.restore();
}

/**
 * 测量线的端点刻度与长度标注。长度用的是原图像素距离，
 * 因此缩放预览和导出显示同一个数字。
 */
function drawMeasureDecoration(
  ctx: CanvasRenderingContext2D,
  annotation: SegmentAnnotation,
  from: Point,
  to: Point,
  scale: number,
) {
  const angle = Math.atan2(to.y - from.y, to.x - from.x);
  const tick = Math.max(6, annotation.size * 2.5) * scale;
  const normal = { x: -Math.sin(angle) * tick, y: Math.cos(angle) * tick };
  ctx.beginPath();
  for (const end of [from, to]) {
    ctx.moveTo(end.x - normal.x, end.y - normal.y);
    ctx.lineTo(end.x + normal.x, end.y + normal.y);
  }
  ctx.stroke();

  const pixels = Math.round(
    Math.hypot(annotation.to.x - annotation.from.x, annotation.to.y - annotation.from.y),
  );
  const label = `${pixels} px`;
  const fontSize = Math.max(12, annotation.size * 3.2) * scale;
  ctx.font = `600 ${fontSize}px system-ui, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "bottom";
  const midX = (from.x + to.x) / 2;
  const midY = (from.y + to.y) / 2 - tick;
  ctx.lineWidth = Math.max(3, annotation.size * scale);
  ctx.strokeStyle = "rgba(0, 0, 0, 0.55)";
  ctx.strokeText(label, midX, midY);
  ctx.fillStyle = annotation.color;
  ctx.fillText(label, midX, midY);
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

function applyRoundedMask(ctx: CanvasRenderingContext2D, width: number, height: number, radius: number) {
  const safeRadius = Math.max(0, Math.min(radius, width / 2, height / 2));
  ctx.save();
  ctx.globalCompositeOperation = "destination-in";
  ctx.beginPath();
  ctx.roundRect(0, 0, width, height, safeRadius);
  ctx.fill();
  ctx.restore();
}
