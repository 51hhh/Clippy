/**
 * 浮动工具条的落点计算：外围优先，外围放不下才进内容内部。
 *
 * 截图选区的工具条和贴图的工具条要的是同一件事——"贴在那块矩形边上、别出屏幕"，
 * 所以这里只写一份，两边各自决定"矩形"是选区还是贴图内容区。思路参考 flashot（MIT）的
 * `computeToolbarPosition` / `computeVerticalToolbarPosition`：候选位置按偏好排序，
 * 第一个装得下的胜出，全都装不下才退到内部。
 *
 * **为什么外围优先**：工具条压在内容上会挡住用户正要看的东西（贴图尤其——它的全部
 * 意义就是那张图）。只有边上确实没地方了（贴图贴在屏幕角上、选区顶满整屏）才进内部。
 */

export type Size = { width: number; height: number };
export type Box = { x: number; y: number; width: number; height: number };

/** 工具条落点。`placement` 说明最后选中了哪个候选，调用方据此改样式（进内部要加底色）。 */
export type ToolbarPlacement = {
  left: number;
  top: number;
  placement: "below" | "above" | "right" | "left" | "inside";
};

/** 工具条与内容、与屏幕边缘的间隙。 */
const GAP = 8;

/**
 * 横排工具条：下方 → 上方 → 内部底边。
 *
 * 水平方向与 `anchor` 右边缘对齐（截图工具条一直是这个习惯），再钳进 `viewport`。
 */
export function horizontalToolbarPlacement(
  anchor: Box,
  toolbar: Size,
  viewport: Size,
  gap = GAP,
): ToolbarPlacement {
  const width = Math.min(toolbar.width, Math.max(0, viewport.width - gap * 2));
  const maxLeft = Math.max(gap, viewport.width - width - gap);
  const left = Math.max(gap, Math.min(anchor.x + anchor.width - width, maxLeft));
  const below = anchor.y + anchor.height + gap;
  const above = anchor.y - toolbar.height - gap;
  const maxTop = Math.max(gap, viewport.height - toolbar.height - gap);
  if (below <= maxTop) return { left, top: below, placement: "below" };
  if (above >= gap) return { left, top: above, placement: "above" };
  // 内部：贴在内容底边上方，仍然钳进视口。
  return {
    left,
    top: clamp(anchor.y + anchor.height - toolbar.height - gap, gap, maxTop),
    placement: "inside",
  };
}

/**
 * 竖排工具条：右侧 → 左侧 → 内部右上。
 *
 * 贴图的工具条是竖排的（`pin.css` 的 `.pin-tools-vertical`），而且它以前是**钉死**在
 * 窗口右上角的：贴图贴在屏幕右边缘时工具条就在屏幕外，一个按钮都点不到。
 */
export function verticalToolbarPlacement(
  anchor: Box,
  toolbar: Size,
  viewport: Size,
  gap = GAP,
): ToolbarPlacement {
  const height = Math.min(toolbar.height, Math.max(0, viewport.height - gap * 2));
  const maxTop = Math.max(gap, viewport.height - height - gap);
  const top = clamp(anchor.y, gap, maxTop);
  const right = anchor.x + anchor.width + gap;
  const left = anchor.x - toolbar.width - gap;
  const maxLeft = Math.max(gap, viewport.width - toolbar.width - gap);
  if (right <= maxLeft) return { left: right, top, placement: "right" };
  if (left >= gap) return { left, top, placement: "left" };
  return {
    left: clamp(anchor.x + anchor.width - toolbar.width - gap, gap, maxLeft),
    top,
    placement: "inside",
  };
}

/**
 * 用户拖过工具条之后的落点：只做钳制，不再自动选边。
 *
 * 一旦用户手动摆过，自动选边就该让位——否则内容尺寸一变（缩放贴图、改选区）工具条
 * 就从用户放的地方跳走。钳制不能省：拖到屏幕外就再也拖不回来了。
 */
export function clampToolbarPosition(
  position: { left: number; top: number },
  toolbar: Size,
  viewport: Size,
  gap = GAP,
): { left: number; top: number } {
  return {
    left: clamp(position.left, gap, Math.max(gap, viewport.width - toolbar.width - gap)),
    top: clamp(position.top, gap, Math.max(gap, viewport.height - toolbar.height - gap)),
  };
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(value, maximum));
}
