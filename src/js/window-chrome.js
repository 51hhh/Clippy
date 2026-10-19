/**
 * 为 Vanilla 页面绑定自定义标题栏行为。
 *
 * 原生窗口动作由调用方从 api.ts 注入；本模块只拥有 DOM 事件和交互过滤，
 * 因而可在 jsdom 中测试，也不会扩大 Tauri IPC 边界。
 */

const INTERACTIVE_SELECTOR = "button,input,select,textarea,a,[role=button],[contenteditable]";

function isInteractiveTarget(target) {
  return target instanceof Element && target.closest(INTERACTIVE_SELECTOR) !== null;
}

function run(action, onError) {
  Promise.resolve()
    .then(action)
    .catch(onError);
}

export function bindWindowChrome({
  titlebar,
  minimizeButton,
  maximizeButton,
  closeButton,
  startDragging,
  minimize,
  toggleMaximize,
  requestClose,
  onError = console.warn,
}) {
  const onPointerDown = (event) => {
    if (event.defaultPrevented || event.button !== 0 || isInteractiveTarget(event.target)) return;
    event.preventDefault();
    run(startDragging, onError);
  };
  const onDoubleClick = (event) => {
    if (event.defaultPrevented || event.button !== 0 || isInteractiveTarget(event.target)) return;
    event.preventDefault();
    run(toggleMaximize, onError);
  };
  const onMinimize = () => run(minimize, onError);
  const onMaximize = () => run(toggleMaximize, onError);
  const onClose = () => run(requestClose, onError);

  titlebar.addEventListener("pointerdown", onPointerDown);
  titlebar.addEventListener("dblclick", onDoubleClick);
  minimizeButton.addEventListener("click", onMinimize);
  maximizeButton.addEventListener("click", onMaximize);
  closeButton.addEventListener("click", onClose);

  return () => {
    titlebar.removeEventListener("pointerdown", onPointerDown);
    titlebar.removeEventListener("dblclick", onDoubleClick);
    minimizeButton.removeEventListener("click", onMinimize);
    maximizeButton.removeEventListener("click", onMaximize);
    closeButton.removeEventListener("click", onClose);
  };
}
