/**
 * 贴图窗口的鼠标/触控板手势规则。
 *
 * 这些事件不能用 React 的 `onWheel` / `onSelect` 属性接：React 17 起把 `wheel`、
 * `touchstart`、`touchmove` 统一注册成**被动**监听器挂在根容器上，于是在处理函数里调
 * `event.preventDefault()` 是空操作（浏览器只会打一条 "Unable to preventDefault inside
 * passive event listener" 的警告）。触控板的捏合手势会被 WebKitGTK 合成成 ctrl+滚轮，
 * 拦不住它就等于让 WebKit 去缩放整个页面——DOM 被放大而窗口不变，内容溢出、工具栏错位。
 * 所以这里自己用 `{ passive: false }` 注册。
 */

/** 一次滚轮事件的含义。`ignore` 表示"吃掉它，什么都不做"。 */
export type PinWheelIntent =
  | { kind: "scale"; delta: number }
  | { kind: "opacity"; delta: number }
  | { kind: "ignore" };

/** 每格滚轮改变的量。贴图缩放与不透明度用同一个步长，手感一致。 */
const WHEEL_STEP = 0.05;

export interface PinWheelEvent {
  deltaY: number;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

/**
 * 滚轮/捏合 → 要做什么。
 *
 * - 光滚轮：改贴图缩放（窗口尺寸随之变化，这是用户要的那种"放大"）。
 * - Shift + 滚轮：改不透明度。以前这挂在 ctrl 上，但 WebKit 把捏合合成成 ctrl+滚轮，
 *   于是两指一捏就顺手把贴图调成半透明——所以让开 ctrl。
 * - Ctrl/Cmd + 滚轮：什么都不做。它要么来自捏合手势，要么是用户想缩放页面，
 *   而贴图窗口按设计就是不可缩放的（页面缩放只会让内容溢出窗口）。
 */
export function pinWheelIntent(event: PinWheelEvent): PinWheelIntent {
  if (event.ctrlKey || event.metaKey) return { kind: "ignore" };
  const delta = event.deltaY > 0 ? -WHEEL_STEP : WHEEL_STEP;
  return event.shiftKey ? { kind: "opacity", delta } : { kind: "scale", delta };
}

/**
 * 这个元素里的文字允许用鼠标划选吗？
 *
 * 文本贴图存在的意义就是让人把内容选走，所以 `<pre>` 与输入框要放行。其余地方一律
 * 禁止：Ubuntu 的强调色是橙色，一次没拦住的划选会把整块内容刷成橙色高亮，
 * 看上去就像"拖动贴图变成了选中图片"。
 */
export function allowsTextSelection(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest("pre, input, textarea, [contenteditable=\"true\"]") !== null;
}

/**
 * 键盘缩放：Ctrl/Cmd 加 `+` `-` `=` `0` 会触发 WebKit 的页面缩放，
 * 和捏合是同一个毛病，所以也要吃掉。
 */
export function isZoomShortcut(event: {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
}): boolean {
  return (event.ctrlKey || event.metaKey) && ["+", "-", "=", "_", "0"].includes(event.key);
}
