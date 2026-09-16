import { useCallback, useEffect, useRef, useState } from "react";
import { clampToolbarPosition, type Size, type ToolbarBounds } from "./toolbarPlacement";

/**
 * 当前有没有任何浮动工具条正在被拖动。
 *
 * 模块级而不是一路 prop 传下去：需要它的是**窗口拖动那条判据**（`pin/gestures.ts`
 * 刻意不跨事件记账，只看每个事件自带的信息），而工具条有两条、各自独立拖动，
 * 把状态从两个组件里提上去再传回 App 只是为了一个布尔值。
 *
 * 这是 pointer capture 之外的**第二道闸**：捕获已经让判据不会误判，但那依赖
 * `setPointerCapture` 真的生效（个别环境会抛）。同一个窗口里同时拖两条工具条是
 * 不可能的（只有一个指针），所以一个布尔就够。
 */
let toolbarDragActive = false;

/** 有工具条正在拖吗？窗口拖动的判据每个 pointermove 问一次。 */
export function isToolbarDragging(): boolean {
  return toolbarDragActive;
}

/**
 * 按住把手拖动浮动工具条。
 *
 * 拖动中的坐标记在 state 里，一旦有值就盖掉自动选边（见 `toolbarPlacement`）——
 * 用户手动摆过之后，内容尺寸变化不该再把工具条挪走。
 *
 * 监听挂在 `window` 上而不是把手上：指针拖出把手（拖得快的时候必然发生）之后
 * 事件就不再经过那个元素了，挂在元素上的话工具条会中途"掉线"。参考 flashot（MIT）
 * 的 `startToolbarDrag`，但用 pointer 事件而不是 mouse 事件，好让触屏与触控板一致。
 *
 * **必须 `setPointerCapture`。** 贴图窗口的"拖动窗口"判据是"主键按着 + 位移够大 +
 * 落点不在 `[data-pin-controls]` 里"（见 `pin/gestures.ts`，它刻意不跨事件记账）。
 * 工具条跟着指针走，指针于是很容易落到工具条外面——那一刻 `event.target` 不再是工具条，
 * 判据当场成立，于是"拖工具条"变成"拖整个贴图窗口"。捕获指针之后所有 pointer 事件的
 * target 都钉在把手上，判据再也不会误判；同时这也是拖动跟手的正解（不依赖指针是否
 * 还悬在某个元素上）。
 */
export function useToolbarDrag(toolbarSize: Size, bounds: ToolbarBounds) {
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const drag = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    left: number;
    top: number;
    /** 捕获指针的那个元素，收尾时要在它上面释放。 */
    target: Element | null;
  } | null>(null);
  // 拖动过程中要读最新的边界做钳制，但不能让它进 effect 的依赖里——那会在拖动途中
  // 重挂监听。用 ref 转一手。
  const latest = useRef({ toolbarSize, bounds });
  latest.current = { toolbarSize, bounds };

  const start = useCallback(
    (event: React.PointerEvent, current: { left: number; top: number }) => {
      // 别让这次按下冒泡成"拖动整个贴图窗口"（`pin/gestures.ts`）或"框选"。
      event.preventDefault();
      event.stopPropagation();
      // 把指针钉在把手上，后续 pointermove 的 target 就不会跑到工具条外面去
      // （那会让窗口拖动的判据误判，见函数头）。捕获失败不致命，只是回到旧行为。
      const target = event.currentTarget;
      try {
        target.setPointerCapture(event.pointerId);
      } catch {
        // jsdom 与个别环境没有这个 API；拖动本身仍然可用。
      }
      drag.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        left: current.left,
        top: current.top,
        target,
      };
      toolbarDragActive = true;
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
      setPosition(clampToolbarPosition(next, latest.current.toolbarSize, latest.current.bounds));
    }
    function onEnd(event: PointerEvent) {
      const state = drag.current;
      if (state?.pointerId !== event.pointerId) return;
      try {
        state.target?.releasePointerCapture(event.pointerId);
      } catch {
        // 已经自动释放（元素被卸载、指针离开设备）时会抛，忽略即可。
      }
      drag.current = null;
      toolbarDragActive = false;
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
      // 拖动途中组件被卸载（画布关掉、贴图关掉）时标志必须清掉，
      // 否则它永久留成 true，窗口从此再也拖不动。
      if (drag.current !== null) {
        drag.current = null;
        toolbarDragActive = false;
      }
    };
  }, []);

  /** 可用范围或工具条尺寸变了（窗口缩放、窗口被拖出屏幕）：把手动位置重新钳一次。 */
  useEffect(() => {
    setPosition((current) => (current ? clampToolbarPosition(current, toolbarSize, bounds) : current));
  }, [toolbarSize.width, toolbarSize.height, bounds.x, bounds.y, bounds.width, bounds.height]);

  return { position, startDrag: start };
}
