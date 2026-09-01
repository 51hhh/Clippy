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
 * 一次事件里主键还按着没有。按键状态是每个事件自带的，不需要跨事件记账。
 */
export function pointerStillHeld(event: { buttons: number }): boolean {
  return (event.buttons & 1) !== 0;
}

/** 拖动起点与按压归属。只有这三样，全部由 `trackDrag*` 两个纯函数推进。 */
export interface DragTracking {
  /** 拖动起点（CSS 坐标）。`null` = 还没有起点。 */
  origin: { x: number; y: number } | null;
  /** 这次按压是从工具条上按下去的：整个按压期间都不许拖窗口。 */
  suppressed: boolean;
  /**
   * 上一次真的发出 `startDragging` 的时刻，用来给"抓不住指针"的情况限速。
   * `-Infinity` = 还没发过，随时可以发。
   */
  startedAt: number;
}

export const NO_DRAG: DragTracking = {
  origin: null,
  suppressed: false,
  startedAt: Number.NEGATIVE_INFINITY,
};

/** 超过这么多 CSS 像素才算拖动，否则一次普通点击就会把窗口挪走。 */
export const DRAG_THRESHOLD = 5;

/**
 * 没等到新的 `pointerdown` 时，两次 `startDragging` 之间的最短间隔。
 *
 * 它只防一件事：合成器**没**接手指针（`startDragging` 失败、或者 X11 上被拒），
 * 于是 `pointermove` 继续送进来、起点一次次重新长出来，把 IPC 刷成每帧一次。
 * 一个真的新按压会带着 `pointerdown` 把它清零（见 `trackDragPointerDown`），
 * 所以这个限速永远不会吃掉用户明确发起的下一次拖动。
 */
const DRAG_RETRY_INTERVAL_MS = 300;

export interface DragPointerEvent {
  buttons: number;
  x: number;
  y: number;
  /** 事件落点是否在 `[data-pin-controls]` 里（工具条、滑块……）。 */
  onControls: boolean;
}

/**
 * `pointerdown`：记起点，或者认出"这次按压属于工具条"。
 *
 * 它是**优化**而不是必需——真正决定能不能拖的是下面的 `trackDragMove`；这个事件被
 * 吞掉时那边照样能从 `pointermove` 里把起点长出来。它同时是"新按压"的唯一确凿证据，
 * 所以顺手把限速清零：一次明确的新按压不该被上一次拖动的冷却挡住。
 */
export function trackDragPointerDown(
  state: DragTracking,
  event: { button: number; x: number; y: number; onControls: boolean },
): DragTracking {
  if (event.button !== 0) return state;
  if (event.onControls) return { ...NO_DRAG, suppressed: true };
  return { ...NO_DRAG, origin: { x: event.x, y: event.y } };
}

/**
 * `pointermove`：判断这一下要不要真的开始拖窗口。
 *
 * **起点可以在这里自己长出来，不必等 `pointerdown`。** 这是"第一下能拖、第二下拖不动、
 * 第三下又能拖"的真正修法：Wayland 上 `startDragging` 之后指针被合成器抓走，这一次的
 * `pointerup` 不会送到 WebKit，于是每次拖完都会留下一件迟到的事情落在**下一次**按压
 * 之后——可能是 `pointercancel`，可能是 `buttons=0` 的收尾 `pointermove`，也可能是
 * WebKit 自己那份"按键还按着"的残留状态把下一个 `pointerdown` 吃掉。三种都是同一个
 * 症状（隔一次拖不动），也都无法靠"再多记一个标记"避开，因为记账本身就是被污染的东西。
 *
 * 所以判据只用每个事件自带的信息：主键按着 + 位移够大 + 落点不在工具条上 = 拖窗口。
 * 迟到的事件最多让这一次多花一个 `pointermove` 重新起点，用户感觉不到。
 */
export function trackDragMove(
  state: DragTracking,
  event: DragPointerEvent,
  now: number,
): { state: DragTracking; start: boolean } {
  // 松手了：把这次按压的一切都忘掉，包括"从工具条按下去"的抑制。
  if (!pointerStillHeld(event)) return { state: NO_DRAG, start: false };
  if (state.suppressed) return { state, start: false };
  if (!state.origin) {
    // 工具条上按住不动地划来划去不该拖窗口；离开工具条之后才认起点。
    if (event.onControls) return { state, start: false };
    return { state: { ...state, origin: { x: event.x, y: event.y } }, start: false };
  }
  const distance = Math.hypot(event.x - state.origin.x, event.y - state.origin.y);
  if (distance < DRAG_THRESHOLD) return { state, start: false };
  // 起点一律清掉：合成器接手之后 WebKit 收不到后续事件，收得到就说明这次没抓住，
  // 那时按限速再试一次比卡住不动好。
  if (now - state.startedAt < DRAG_RETRY_INTERVAL_MS) {
    return { state: { ...state, origin: null }, start: false };
  }
  return { state: { ...state, origin: null, startedAt: now }, start: true };
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
