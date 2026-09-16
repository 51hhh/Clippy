import { useMemo, useRef, useState } from "react";
import {
  clampRect,
  committedSelection,
  contains,
  coversBounds,
  hitHandle,
  hoverCandidate,
  moveRect,
  normalizeRect,
  resizeRect,
  windowAt,
} from "./geometry";
import type { Point, Rect, ResizeHandle, WindowCandidate } from "./types";

type Interaction =
  | { kind: "create"; start: Point; candidate: WindowCandidate | null }
  | { kind: "move"; start: Point; initial: Rect }
  | { kind: "resize"; start: Point; initial: Rect; handle: ResizeHandle }
  | null;

export function useSelection(width: number, height: number, windows: WindowCandidate[]) {
  const bounds = useMemo(() => ({ x: 0, y: 0, width, height }), [height, width]);
  const interaction = useRef<Interaction>(null);
  const [selection, setSelection] = useState<Rect | null>(null);
  const [candidate, setCandidate] = useState<WindowCandidate | null>(null);

  /**
   * 铺满全屏的选区没有可移动的余量：此时"选区内部"要让位给重新框选，
   * 否则点一下取了全屏之后就再也框不出小区域了。手柄不受影响，始终可拖。
   */
  const movable = selection && !coversBounds(selection, bounds) ? selection : null;

  /** 供调用方判断这次按下是否落在缩放手柄上（决定指针事件归选区还是归画布）。 */
  function handleAt(point: Point): ResizeHandle | null {
    return selection ? hitHandle(selection, point) : null;
  }

  function pointerDown(point: Point) {
    const handle = handleAt(point);
    if (handle && selection) {
      interaction.current = { kind: "resize", start: point, initial: selection, handle };
      return;
    }
    if (movable && contains(movable, point)) {
      interaction.current = { kind: "move", start: point, initial: movable };
      return;
    }
    const hovered = windowAt(windows, point);
    interaction.current = { kind: "create", start: point, candidate: hovered };
    setSelection({ x: point.x, y: point.y, width: 0, height: 0 });
  }

  function pointerMove(point: Point) {
    const active = interaction.current;
    if (!active) {
      setCandidate(hoverCandidate(windows, point, movable));
      return;
    }
    const delta = { x: point.x - active.start.x, y: point.y - active.start.y };
    if (active.kind === "create") {
      setSelection(clampRect(normalizeRect(active.start, point), bounds, 0));
    } else if (active.kind === "move") {
      setSelection(moveRect(active.initial, delta, bounds));
    } else {
      setSelection(resizeRect(active.initial, active.handle, delta, bounds));
    }
  }

  /** 返回本次新框出的选区（调整已有选区或空手松开时返回 null）。 */
  function pointerUp(point: Point): Rect | null {
    const active = interaction.current;
    interaction.current = null;
    setCandidate(null);
    if (!active || active.kind !== "create") return null;
    const committed = committedSelection(active.start, point, active.candidate, bounds);
    setSelection(committed);
    return committed;
  }

  /** 回到"还没框选"的状态（右键取消选区）。 */
  function reset() {
    interaction.current = null;
    setCandidate(null);
    setSelection(null);
  }

  return { selection, candidate, bounds, handleAt, pointerDown, pointerMove, pointerUp, reset };
}
