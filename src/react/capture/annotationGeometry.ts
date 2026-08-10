import type { Annotation, Point, Rect } from "./types";

export function annotationBounds(annotation: Annotation): Rect {
  if (annotation.type === "rect" || annotation.type === "blur" || annotation.type === "mosaic") {
    return annotation.rect;
  }
  if (annotation.type === "arrow") {
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
  if (annotation.type === "pen") {
    return { ...annotation, points: annotation.points.map(move) };
  }
  if (annotation.type === "rect" || annotation.type === "blur" || annotation.type === "mosaic") {
    return { ...annotation, rect: { ...annotation.rect, x: annotation.rect.x + delta.x, y: annotation.rect.y + delta.y } };
  }
  if (annotation.type === "arrow") {
    return { ...annotation, from: move(annotation.from), to: move(annotation.to) };
  }
  return { ...annotation, at: move(annotation.at) };
}

function hitAnnotation(annotation: Annotation, point: Point): boolean {
  const bounds = annotationBounds(annotation);
  const padding = Math.max(6, "size" in annotation ? annotation.size * 1.5 : 6);
  if (point.x < bounds.x - padding || point.x > bounds.x + bounds.width + padding) return false;
  if (point.y < bounds.y - padding || point.y > bounds.y + bounds.height + padding) return false;
  if (annotation.type === "arrow") {
    return distanceToSegment(point, annotation.from, annotation.to) <= padding;
  }
  if (annotation.type === "pen") {
    return annotation.points.some((item, index) => {
      const next = annotation.points[index + 1];
      return next ? distanceToSegment(point, item, next) <= padding : false;
    });
  }
  return true;
}

function distanceToSegment(point: Point, start: Point, end: Point): number {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return Math.hypot(point.x - start.x, point.y - start.y);
  const t = Math.max(0, Math.min(1, ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared));
  return Math.hypot(point.x - (start.x + t * dx), point.y - (start.y + t * dy));
}
