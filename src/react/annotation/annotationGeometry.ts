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
    return annotation.rect;
  }
  if (hasEndpoints(annotation)) {
    return {
      x: Math.min(annotation.from.x, annotation.to.x),
      y: Math.min(annotation.from.y, annotation.to.y),
      width: Math.abs(annotation.to.x - annotation.from.x),
      height: Math.abs(annotation.to.y - annotation.from.y),
    };
  }
  if (annotation.type === "text") {
    const fontSize = Math.max(14, annotation.size * 4);
    return {
      x: annotation.at.x,
      y: annotation.at.y,
      width: Math.max(fontSize, annotation.text.length * fontSize * 0.62),
      height: fontSize * 1.25,
    };
  }
  const xs = annotation.points.map((point) => point.x);
  const ys = annotation.points.map((point) => point.y);
  return {
    x: Math.min(...xs),
    y: Math.min(...ys),
    width: Math.max(1, Math.max(...xs) - Math.min(...xs)),
    height: Math.max(1, Math.max(...ys) - Math.min(...ys)),
  };
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
    return Math.abs(ellipseDistance(point, bounds)) <= padding;
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
