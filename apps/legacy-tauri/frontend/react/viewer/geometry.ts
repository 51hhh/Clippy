import type { Point } from "../annotation/types";

export type Size = { width: number; height: number };
export type View = { scale: number; x: number; y: number; mode: "fit" | "actual" | "custom" };
export const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
export function boundedDpr(dpr: number, size: Size): number {
  return Math.max(.25, Math.min(Number.isFinite(dpr) ? dpr : 1, 8192 / Math.max(size.width, size.height, 1), Math.sqrt(16_777_216 / Math.max(1, size.width * size.height))));
}
export function fitView(size: Size, image: Size): View {
  const bottomSpace = Math.min(160, size.height * .4);
  return { scale: Math.max(.005, Math.min((size.width - 40) / image.width, (size.height - bottomSpace) / image.height)), x: 0, y: -Math.min(28, size.height * .12), mode: "fit" };
}
export function imageOrigin(view: View, size: Size, image: Size): Point {
  return { x: size.width / 2 + view.x - image.width * view.scale / 2, y: size.height / 2 + view.y - image.height * view.scale / 2 };
}
export function sourcePoint(point: Point, view: View, size: Size, image: Size, bounded = false): Point | null {
  const origin = imageOrigin(view, size, image);
  const x = (point.x - origin.x) / view.scale, y = (point.y - origin.y) / view.scale;
  if (!bounded && (x < 0 || y < 0 || x >= image.width || y >= image.height)) return null;
  return { x: clamp(x, 0, image.width), y: clamp(y, 0, image.height) };
}
export function zoomAt(view: View, scale: number, point: Point, size: Size, _image: Size): View {
  const nextScale = clamp(scale, .005, 16), ratio = nextScale / view.scale;
  const anchor = { x: point.x - size.width / 2, y: point.y - size.height / 2 };
  return { scale: nextScale, x: anchor.x - (anchor.x - view.x) * ratio, y: anchor.y - (anchor.y - view.y) * ratio, mode: "custom" };
}
