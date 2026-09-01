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

/**
 * 工具条可以待的范围，**窗口局部坐标**。
 *
 * 不是 `window.innerWidth/innerHeight`，这一点是这个模块最容易搞错的地方，而且我搞错过：
 * 贴图窗口的外框恒等于「内容 + 12×2 阴影 + 44 控件栏」，也就是**永远给工具条留够了
 * 位置**。拿窗口自己当边界的话，右侧候选永远装得下、一次都不会翻边，"超出屏幕自动调整"
 * 于是完全不生效——而真正会超出的是**窗口在屏幕上**的位置（用户把贴图拖到屏幕边缘）。
 *
 * 所以这个矩形要由后端给：窗口矩形与显示器工作区的交集，换算到窗口局部坐标
 * （见 `pin::commands::get_pin_bounds`）。窗口完全在屏内时它就等于整个窗口，
 * 那时行为和以前一致。
 */
export type ToolbarBounds = Box;

/** 窗口完全在屏幕内时的边界：整个窗口。 */
export function fullWindowBounds(size: Size): ToolbarBounds {
  return { x: 0, y: 0, width: size.width, height: size.height };
}

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
  bounds: ToolbarBounds,
  gap = GAP,
): ToolbarPlacement {
  const minLeft = bounds.x + gap;
  const minTop = bounds.y + gap;
  const width = Math.min(toolbar.width, Math.max(0, bounds.width - gap * 2));
  const maxLeft = Math.max(minLeft, bounds.x + bounds.width - width - gap);
  const left = clamp(anchor.x + anchor.width - width, minLeft, maxLeft);
  const below = anchor.y + anchor.height + gap;
  const above = anchor.y - toolbar.height - gap;
  const maxTop = Math.max(minTop, bounds.y + bounds.height - toolbar.height - gap);
  if (below <= maxTop) return { left, top: below, placement: "below" };
  if (above >= minTop) return { left, top: above, placement: "above" };
  // 内部：贴在内容底边上方，仍然钳进可用范围。
  return {
    left,
    top: clamp(anchor.y + anchor.height - toolbar.height - gap, minTop, maxTop),
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
  bounds: ToolbarBounds,
  gap = GAP,
): ToolbarPlacement {
  const minLeft = bounds.x + gap;
  const minTop = bounds.y + gap;
  const height = Math.min(toolbar.height, Math.max(0, bounds.height - gap * 2));
  const maxTop = Math.max(minTop, bounds.y + bounds.height - height - gap);
  const top = clamp(anchor.y, minTop, maxTop);
  const right = anchor.x + anchor.width + gap;
  const left = anchor.x - toolbar.width - gap;
  const maxLeft = Math.max(minLeft, bounds.x + bounds.width - toolbar.width - gap);
  if (right <= maxLeft) return { left: right, top, placement: "right" };
  if (left >= minLeft) return { left, top, placement: "left" };
  return {
    left: clamp(anchor.x + anchor.width - toolbar.width - gap, minLeft, maxLeft),
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
  bounds: ToolbarBounds,
  gap = GAP,
): { left: number; top: number } {
  const minLeft = bounds.x + gap;
  const minTop = bounds.y + gap;
  return {
    left: clamp(position.left, minLeft, Math.max(minLeft, bounds.x + bounds.width - toolbar.width - gap)),
    top: clamp(position.top, minTop, Math.max(minTop, bounds.y + bounds.height - toolbar.height - gap)),
  };
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(value, maximum));
}
