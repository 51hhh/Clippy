/**
 * 图片 OCR 内翻译视图的宿主桥接。
 * React 根始终留在静态宿主，仅把内容 portal 到当前图片的 slot；清空预览不会销毁根。
 */
export function createImageTranslationView() {
  /** @type {{ clipId: number, container: HTMLElement, visible: boolean } | null} */
  let snapshot = null;
  let binding = null;
  const listeners = new Set();
  const publish = (next) => {
    snapshot = next;
    listeners.forEach((listener) => listener());
  };
  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    getSnapshot: () => snapshot,
    bind({ clipId, container, isCurrent, onVisibilityChange }) {
      binding = { isCurrent, onVisibilityChange };
      publish({ clipId, container, visible: false });
    },
    clear() {
      binding = null;
      if (snapshot !== null) publish(null);
    },
    show() {
      if (!snapshot || !binding?.isCurrent()) return false;
      binding.onVisibilityChange(true);
      if (!snapshot.visible) publish({ ...snapshot, visible: true });
      return true;
    },
    hide(container) {
      if (!snapshot || snapshot.container !== container || !binding?.isCurrent()) return;
      binding.onVisibilityChange(false);
      if (snapshot.visible) publish({ ...snapshot, visible: false });
    },
  };
}

export const imageTranslationView = createImageTranslationView();

/** 仅打开 OCR 区内的翻译控件；翻译请求仍由用户另行明确执行。 */
export function revealImageTranslation(panel, view = imageTranslationView) {
  const container = view.getSnapshot()?.container;
  if (!container || !panel?.contains(container) || !view.show()) return false;
  const area = container.closest(".preview-ocr-result");
  const backButton = area?.querySelector(".preview-ocr-back");
  if (backButton instanceof HTMLElement) backButton.focus({ preventScroll: true });
  const scroll = panel.querySelector(".preview-scroll");
  if (scroll && area) {
    // 仅调整侧栏的外层滚动，不滚动宿主页面，也不为正文创建内层滚动容器。
    const top = scroll.scrollTop + area.getBoundingClientRect().top
      - scroll.getBoundingClientRect().top - scroll.clientTop;
    scroll.scrollTo?.({ top, behavior: "smooth" });
  }
  return true;
}
