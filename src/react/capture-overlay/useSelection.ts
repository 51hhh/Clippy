import { useMemo, useRef, useState } from "react";
import {
  clampRect,
  committedSelection,
  contains,
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

  function pointerDown(point: Point) {
    if (selection) {
      const handle = hitHandle(selection, point);
      if (handle) {
        interaction.current = { kind: "resize", start: point, initial: selection, handle };
        return;
      }
      if (contains(selection, point)) {
        interaction.current = { kind: "move", start: point, initial: selection };
        return;
      }
    }
    const hovered = windowAt(windows, point);
    interaction.current = { kind: "create", start: point, candidate: hovered };
    setSelection({ x: point.x, y: point.y, width: 0, height: 0 });
  }

  function pointerMove(point: Point) {
    const active = interaction.current;
    if (!active) {
      setCandidate(hoverCandidate(windows, point, selection));
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

  /** 返回本次新框出的选区（调整已有选区或空手松开时返回 null），调用方据此决定是否直接进编辑器。 */
  function pointerUp(point: Point): Rect | null {
    const active = interaction.current;
    interaction.current = null;
    setCandidate(null);
    if (!active || active.kind !== "create") return null;
    const committed = committedSelection(active.start, point, active.candidate, bounds);
    setSelection(committed);
    return committed;
  }

  return { selection, candidate, setSelection, pointerDown, pointerMove, pointerUp };
}
