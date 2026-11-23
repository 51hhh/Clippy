import {
  EFFECT_TYPES,
  type Annotation,
  type EffectAnnotation,
  type Point,
  type Rect,
  type SegmentAnnotation,
  type ShapeAnnotation,
  type StrokeAnnotation,
  type VectorAnnotation,
} from "./types";

const EFFECT_TYPE_SET: ReadonlySet<string> = new Set(EFFECT_TYPES);

export function isEffectAnnotation(annotation: Annotation): annotation is EffectAnnotation {
  return EFFECT_TYPE_SET.has(annotation.type);
}

export function isVectorAnnotation(annotation: Annotation): annotation is VectorAnnotation {
  return !isEffectAnnotation(annotation);
}

/** 由矩形定义的注解（图形与效果共用同一套包围盒、移动和命中逻辑） */
export function hasRect(annotation: Annotation): annotation is ShapeAnnotation | EffectAnnotation {
  return "rect" in annotation;
}

/** 两点线段类注解 */
export function hasEndpoints(annotation: Annotation): annotation is SegmentAnnotation {
  return "from" in annotation;
}

/** 折线类注解 */
export function hasPoints(annotation: Annotation): annotation is StrokeAnnotation {
  return "points" in annotation;
}

export function annotationBounds(annotation: Annotation): Rect {
  if (hasRect(annotation)) {
    if (!("size" in annotation) || annotation.type === "highlight") return annotation.rect;
    return expandRect(annotation.rect, annotation.size / 2);
  }
  if (hasEndpoints(annotation)) {
    const points = [annotation.from, annotation.to];
    if (annotation.type === "arrow") points.push(...arrowHeadPoints(annotation));
    let bounds = pointsBounds(points);
    if (annotation.type === "measure") bounds = unionRect(bounds, measureDecorationBounds(annotation));
    return expandRect(bounds, annotation.size / 2);
  }
  if (annotation.type === "text") {
    const fontSize = Math.max(14, annotation.size * 4);
    const stroke = Math.max(3, annotation.size) / 2;
    return expandRect({
      x: annotation.at.x,
      y: annotation.at.y,
      width: Math.max(fontSize, estimatedTextWidth(annotation.text, fontSize)),
      height: fontSize * 1.25,
    }, stroke);
  }
  if (annotation.points.length === 0) {
    return { x: 0, y: 0, width: 0, height: 0 };
  }
  const width = annotation.type === "marker"
    ? Math.max(2, annotation.size * 2.6)
    : Math.max(1, annotation.size);
  return expandRect(pointsBounds(annotation.points), width / 2);
}

function pointsBounds(points: Point[]): Rect {
  const xs = points.map((point) => point.x);
  const ys = points.map((point) => point.y);
  return {
    x: Math.min(...xs),
    y: Math.min(...ys),
    width: Math.max(1, Math.max(...xs) - Math.min(...xs)),
    height: Math.max(1, Math.max(...ys) - Math.min(...ys)),
  };
}

function expandRect(rect: Rect, padding: number): Rect {
  return {
    x: rect.x - padding,
    y: rect.y - padding,
    width: rect.width + padding * 2,
    height: rect.height + padding * 2,
  };
}

function unionRect(left: Rect, right: Rect): Rect {
  const x = Math.min(left.x, right.x);
  const y = Math.min(left.y, right.y);
  const x1 = Math.max(left.x + left.width, right.x + right.width);
  const y1 = Math.max(left.y + left.height, right.y + right.height);
  return { x, y, width: x1 - x, height: y1 - y };
}

function arrowHeadPoints(annotation: SegmentAnnotation): Point[] {
  const angle = Math.atan2(annotation.to.y - annotation.from.y, annotation.to.x - annotation.from.x);
  const length = Math.max(10, annotation.size * 4);
  return [-1, 1].map((side) => ({
    x: annotation.to.x - length * Math.cos(angle + side * Math.PI / 7),
    y: annotation.to.y - length * Math.sin(angle + side * Math.PI / 7),
  }));
}

function measureDecorationBounds(annotation: SegmentAnnotation): Rect {
  const angle = Math.atan2(annotation.to.y - annotation.from.y, annotation.to.x - annotation.from.x);
  const tick = Math.max(6, annotation.size * 2.5);
  const normal = { x: -Math.sin(angle) * tick, y: Math.cos(angle) * tick };
  const tickBounds = pointsBounds([
    { x: annotation.from.x - normal.x, y: annotation.from.y - normal.y },
    { x: annotation.from.x + normal.x, y: annotation.from.y + normal.y },
    { x: annotation.to.x - normal.x, y: annotation.to.y - normal.y },
    { x: annotation.to.x + normal.x, y: annotation.to.y + normal.y },
  ]);
  const fontSize = Math.max(14, annotation.size * 3.2);
  const label = `${Math.round(Math.hypot(
    annotation.to.x - annotation.from.x,
    annotation.to.y - annotation.from.y,
  ))} px`;
  const center = {
    x: (annotation.from.x + annotation.to.x) / 2,
    y: (annotation.from.y + annotation.to.y) / 2 - tick,
  };
  const labelWidth = estimatedTextWidth(label, fontSize);
  const labelBounds = {
    x: center.x - labelWidth / 2,
    y: center.y - fontSize,
    width: labelWidth,
    height: fontSize,
  };
  return unionRect(tickBounds, labelBounds);
}

function estimatedTextWidth(text: string, fontSize: number): number {
  return Array.from(text).reduce((width, character) => {
    if (/\s/u.test(character)) return width + fontSize * 0.33;
    // CJK、全角字符和 emoji 在固定 CJK 字体里大致占一个 em；拉丁字符按 0.62 em。
    return width + fontSize * (/[^\u0000-\u024f]/u.test(character) ? 1 : 0.62);
  }, 0);
}

export function annotationAt(annotations: Annotation[], point: Point): Annotation | null {
  for (let index = annotations.length - 1; index >= 0; index -= 1) {
    const annotation = annotations[index];
    if (hitAnnotation(annotation, point)) return annotation;
  }
  return null;
}

export function translateAnnotation(annotation: Annotation, delta: Point): Annotation {
  const move = (point: Point) => ({ x: point.x + delta.x, y: point.y + delta.y });
  if (hasPoints(annotation)) {
    return { ...annotation, points: annotation.points.map(move) };
  }
  if (hasRect(annotation)) {
    return {
      ...annotation,
      rect: { ...annotation.rect, x: annotation.rect.x + delta.x, y: annotation.rect.y + delta.y },
    };
  }
  if (hasEndpoints(annotation)) {
    return { ...annotation, from: move(annotation.from), to: move(annotation.to) };
  }
  return { ...annotation, at: move(annotation.at) };
}

function hitAnnotation(annotation: Annotation, point: Point): boolean {
  const bounds = annotationBounds(annotation);
  const padding = Math.max(6, "size" in annotation ? annotation.size * 1.5 : 6);
  if (point.x < bounds.x - padding || point.x > bounds.x + bounds.width + padding) return false;
  if (point.y < bounds.y - padding || point.y > bounds.y + bounds.height + padding) return false;
  if (hasEndpoints(annotation)) {
    return distanceToSegment(point, annotation.from, annotation.to) <= padding;
  }
  if (hasPoints(annotation)) {
    // 折线注解按线段命中：marker 更粗，所以判定半径跟着线宽走。
    const reach = Math.max(padding, annotation.size * (annotation.type === "marker" ? 1.6 : 0.6));
    return annotation.points.some((item, index) => {
      const next = annotation.points[index + 1];
      return next ? distanceToSegment(point, item, next) <= reach : false;
    });
  }
  if (annotation.type === "ellipse") {
    // 椭圆只在轮廓附近命中，否则空心图形会挡住底下的注解。
    return Math.abs(ellipseDistance(point, annotation.rect)) <= padding;
  }
  return true;
}

/**
 * 点到椭圆轮廓的近似距离（负=在内部）。用归一化半径差乘上局部半径，
 * 精度对命中判定足够，而且不需要迭代求最近点。
 */
function ellipseDistance(point: Point, bounds: Rect): number {
  const rx = Math.max(bounds.width / 2, 0.001);
  const ry = Math.max(bounds.height / 2, 0.001);
  const dx = (point.x - (bounds.x + rx)) / rx;
  const dy = (point.y - (bounds.y + ry)) / ry;
  const normalized = Math.hypot(dx, dy);
  return (normalized - 1) * Math.min(rx, ry);
}

function distanceToSegment(point: Point, start: Point, end: Point): number {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return Math.hypot(point.x - start.x, point.y - start.y);
  const t = Math.max(0, Math.min(1, ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared));
  return Math.hypot(point.x - (start.x + t * dx), point.y - (start.y + t * dy));
}
