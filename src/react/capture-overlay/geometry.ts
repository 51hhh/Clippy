import type { Point, Rect, ResizeHandle, WindowCandidate } from "./types";

export function normalizeRect(start: Point, end: Point): Rect {
  return {
    x: Math.min(start.x, end.x),
    y: Math.min(start.y, end.y),
    width: Math.abs(end.x - start.x),
    height: Math.abs(end.y - start.y),
  };
}

export function clampRect(rect: Rect, bounds: Rect, minimum = 2): Rect {
  const width = Math.max(minimum, Math.min(rect.width, bounds.width));
  const height = Math.max(minimum, Math.min(rect.height, bounds.height));
  return {
    x: Math.max(bounds.x, Math.min(rect.x, bounds.x + bounds.width - width)),
    y: Math.max(bounds.y, Math.min(rect.y, bounds.y + bounds.height - height)),
    width,
    height,
  };
}

export function contains(rect: Rect, point: Point): boolean {
  return (
    point.x >= rect.x &&
    point.x <= rect.x + rect.width &&
    point.y >= rect.y &&
    point.y <= rect.y + rect.height
  );
}

export function windowAt(windows: WindowCandidate[], point: Point): WindowCandidate | null {
  return windows.find((candidate) => contains(candidate, point)) || null;
}

/**
 * 松手时落地的选区：几乎没拖动就当成点击，用悬停窗口速选；
 * 否则用拖出的矩形，小到没意义就作废（返回 null 表示这次框选不成立）。
 */
export function committedSelection(
  start: Point,
  end: Point,
  candidate: WindowCandidate | null,
  bounds: Rect,
): Rect | null {
  if (Math.hypot(end.x - start.x, end.y - start.y) < 4) {
    return candidate ? clampRect(candidate, bounds) : null;
  }
  const dragged = clampRect(normalizeRect(start, end), bounds, 0);
  return dragged.width >= 2 && dragged.height >= 2 ? dragged : null;
}

/**
 * 悬停时该高亮哪个窗口。选区内部让位给移动/缩放手势，选区外面继续提示可速选的窗口，
 * 否则用户随手框过一次之后就再也用不上窗口速选了。
 */
export function hoverCandidate(
  windows: WindowCandidate[],
  point: Point,
  selection: Rect | null,
): WindowCandidate | null {
  if (selection && contains(selection, point)) return null;
  return windowAt(windows, point);
}

export function moveRect(rect: Rect, delta: Point, bounds: Rect): Rect {
  return clampRect({ ...rect, x: rect.x + delta.x, y: rect.y + delta.y }, bounds);
}

export function resizeRect(
  rect: Rect,
  handle: ResizeHandle,
  delta: Point,
  bounds: Rect,
  minimum = 8,
): Rect {
  let left = rect.x;
  let top = rect.y;
  let right = rect.x + rect.width;
  let bottom = rect.y + rect.height;
  if (handle.includes("w")) left += delta.x;
  if (handle.includes("e")) right += delta.x;
  if (handle.includes("n")) top += delta.y;
  if (handle.includes("s")) bottom += delta.y;
  if (right - left < minimum) {
    if (handle.includes("w")) left = right - minimum;
    else right = left + minimum;
  }
  if (bottom - top < minimum) {
    if (handle.includes("n")) top = bottom - minimum;
    else bottom = top + minimum;
  }
  left = Math.max(bounds.x, left);
  top = Math.max(bounds.y, top);
  right = Math.min(bounds.x + bounds.width, right);
  bottom = Math.min(bounds.y + bounds.height, bottom);
  return { x: left, y: top, width: right - left, height: bottom - top };
}

export function hitHandle(rect: Rect, point: Point, radius = 9): ResizeHandle | null {
  const x = [rect.x, rect.x + rect.width / 2, rect.x + rect.width];
  const y = [rect.y, rect.y + rect.height / 2, rect.y + rect.height];
  const handles: Array<[ResizeHandle, number, number]> = [
    ["nw", x[0], y[0]], ["n", x[1], y[0]], ["ne", x[2], y[0]],
    ["e", x[2], y[1]], ["se", x[2], y[2]], ["s", x[1], y[2]],
    ["sw", x[0], y[2]], ["w", x[0], y[1]],
  ];
  return handles.find(([, hx, hy]) => Math.hypot(point.x - hx, point.y - hy) <= radius)?.[0] || null;
}
