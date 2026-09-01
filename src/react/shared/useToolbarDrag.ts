import { useCallback, useEffect, useRef, useState } from "react";
import { clampToolbarPosition, type Size } from "./toolbarPlacement";

/**
 * 按住把手拖动浮动工具条。
 *
 * 拖动中的坐标记在 state 里，一旦有值就盖掉自动选边（见 `toolbarPlacement`）——
 * 用户手动摆过之后，内容尺寸变化不该再把工具条挪走。
 *
 * 监听挂在 `window` 上而不是把手上：指针拖出把手（拖得快的时候必然发生）之后
 * 事件就不再经过那个元素了，挂在元素上的话工具条会中途"掉线"。参考 flashot（MIT）
 * 的 `startToolbarDrag`，但用 pointer 事件而不是 mouse 事件，好让触屏与触控板一致。
 */
export function useToolbarDrag(toolbarSize: Size, viewport: Size) {
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const drag = useRef<{ pointerId: number; startX: number; startY: number; left: number; top: number } | null>(null);
  // 拖动过程中要读最新的尺寸做钳制，但不能让它进 effect 的依赖里——那会在拖动途中
  // 重挂监听。用 ref 转一手。
  const bounds = useRef({ toolbarSize, viewport });
  bounds.current = { toolbarSize, viewport };

  const start = useCallback(
    (event: React.PointerEvent, current: { left: number; top: number }) => {
      // 别让这次按下冒泡成"拖动整个贴图窗口"（`pin/gestures.ts`）或"框选"。
      event.preventDefault();
      event.stopPropagation();
      drag.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        left: current.left,
        top: current.top,
      };
      setPosition(current);
    },
    [],
  );

  useEffect(() => {
    function onMove(event: PointerEvent) {
      const state = drag.current;
      if (!state || event.pointerId !== state.pointerId) return;
      const next = {
        left: state.left + (event.clientX - state.startX),
        top: state.top + (event.clientY - state.startY),
      };
      setPosition(clampToolbarPosition(next, bounds.current.toolbarSize, bounds.current.viewport));
    }
    function onEnd(event: PointerEvent) {
      if (drag.current?.pointerId === event.pointerId) drag.current = null;
    }
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd);
    // 系统接手手势（触控板三指、窗口失焦）时 pointerup 不一定来，靠 cancel 收尾，
    // 否则 drag 一直留着，下一次 pointermove 会把工具条从旧起点跳走。
    window.addEventListener("pointercancel", onEnd);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
      window.removeEventListener("pointercancel", onEnd);
    };
  }, []);

  /** 视口或工具条尺寸变了（窗口缩放）：把手动位置重新钳一次，别留在屏幕外。 */
  useEffect(() => {
    setPosition((current) => (current ? clampToolbarPosition(current, toolbarSize, viewport) : current));
  }, [toolbarSize.width, toolbarSize.height, viewport.width, viewport.height]);

  return { position, startDrag: start, dragging: drag.current !== null };
}
