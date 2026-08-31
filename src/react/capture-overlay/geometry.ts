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

/**
 * 光标下的窗口。
 *
 * 后端下发的候选数组即堆叠顺序（索引 0 最上层），所以取第一个包含光标的候选，
 * 选到的就是肉眼看到的那个窗口——被完全遮住的窗口自然轮不到。
 * 不能按面积挑：一个大窗口压在小窗口上时，"最小的赢"会选中看不见的那个。
 */
export function windowAt(windows: WindowCandidate[], point: Point): WindowCandidate | null {
  return windows.find((candidate) => contains(candidate, point)) || null;
}

/** 几乎没有位移就算点击，而不是一次面积为零的拖拽。 */
const CLICK_SLOP = 4;

/**
 * 松手时落地的选区。
 *
 * - 拖出来的矩形原样落地（钳进屏幕边界），小到没面积就作废（返回 null）。
 * - 几乎没动就是点击：鼠标停在某个窗口上就速选那个窗口，
 *   停在空地上则取整个显示器——参考项目里"直接点一下就是全屏"的手感。
 */
export function committedSelection(
  start: Point,
  end: Point,
  candidate: WindowCandidate | null,
  bounds: Rect,
): Rect | null {
  if (Math.hypot(end.x - start.x, end.y - start.y) < CLICK_SLOP) {
    return clampRect(candidate ?? bounds, bounds);
  }
  const dragged = clampRect(normalizeRect(start, end), bounds, 0);
  return dragged.width >= 2 && dragged.height >= 2 ? dragged : null;
}

/**
 * 选区是否已经铺满整个显示器。铺满时"选区内部拖拽"没有可移动的余量，
 * 必须让位给重新框选，否则点一下取了全屏之后就再也框不出小区域了。
 */
export function coversBounds(rect: Rect, bounds: Rect, tolerance = 1): boolean {
  return (
    rect.x - bounds.x <= tolerance &&
    rect.y - bounds.y <= tolerance &&
    bounds.x + bounds.width - (rect.x + rect.width) <= tolerance &&
    bounds.y + bounds.height - (rect.y + rect.height) <= tolerance
  );
}

/**
 * 逻辑坐标的选区换算成冻结帧的像素坐标——标注和导出都在像素空间里做。
 * 取整后仍钳在帧内，避免 `renderExport` 采样越界。
 */
export function toPixelRect(rect: Rect, scaleX: number, scaleY: number, frame: Rect): Rect {
  const left = Math.round(rect.x * scaleX);
  const top = Math.round(rect.y * scaleY);
  const right = Math.round((rect.x + rect.width) * scaleX);
  const bottom = Math.round((rect.y + rect.height) * scaleY);
  const clamped = {
    x: Math.max(frame.x, Math.min(left, frame.x + frame.width)),
    y: Math.max(frame.y, Math.min(top, frame.y + frame.height)),
    width: 0,
    height: 0,
  };
  clamped.width = Math.max(1, Math.min(right, frame.x + frame.width) - clamped.x);
  clamped.height = Math.max(1, Math.min(bottom, frame.y + frame.height) - clamped.y);
  return clamped;
}

/**
 * 工具条落点：优先贴在选区下方，放不下就翻到上方，两边都放不下就压在屏幕底部。
 * 水平方向与选区右边缘对齐，再钳进视口，宽度超过视口时直接靠左。
 */
export function toolbarPlacement(
  selection: Rect,
  toolbar: { width: number; height: number },
  viewport: { width: number; height: number },
  margin = 8,
): { left: number; top: number } {
  const width = Math.min(toolbar.width, Math.max(0, viewport.width - margin * 2));
  const maxLeft = Math.max(margin, viewport.width - width - margin);
  const left = Math.max(margin, Math.min(selection.x + selection.width - width, maxLeft));
  const below = selection.y + selection.height + margin;
  const above = selection.y - toolbar.height - margin;
  const maxTop = Math.max(margin, viewport.height - toolbar.height - margin);
  if (below <= maxTop) return { left, top: below };
  if (above >= margin) return { left, top: above };
  return { left, top: maxTop };
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
