import { useMemo, useRef, useState } from "react";
import {
  clampRect,
  contains,
  hitHandle,
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
      setCandidate(selection ? null : windowAt(windows, point));
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

  function pointerUp(point: Point) {
    const active = interaction.current;
    interaction.current = null;
    if (!active) return;
    if (active.kind === "create") {
      const distance = Math.hypot(point.x - active.start.x, point.y - active.start.y);
      if (distance < 4 && active.candidate) {
        setSelection(clampRect(active.candidate, bounds));
      } else {
        setSelection((current) => current && current.width >= 2 && current.height >= 2 ? current : null);
      }
    }
    setCandidate(null);
  }

  return { selection, candidate, setSelection, pointerDown, pointerMove, pointerUp };
}
