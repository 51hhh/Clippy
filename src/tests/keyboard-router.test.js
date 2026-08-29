import { describe, it, expect, beforeEach, vi } from "vitest";
import { createKeyboardRouter } from "../js/keyboard-router.js";

/** 只实现路由用到的那几个方法，其余由断言覆盖 */
function fakeList({ mode = "all", searchVisible = false, expanded = false } = {}) {
  return {
    getFocusedClip: vi.fn(() => ({ id: 7 })),
    getPanelMode: () => mode,
    canExpandHere: () => false,
    expandRowActions: vi.fn(),
    collapseActions: vi.fn(),
    hasExpanded: () => expanded,
    moveRow: vi.fn(),
    moveCol: vi.fn(),
    activateFocus: vi.fn(),
    selectByIndex: vi.fn().mockResolvedValue(true),
    search: {
      isVisible: () => searchVisible,
      dismissStage: vi.fn(() => "panel"),
    },
  };
}

function fakePreview(initialVisible = false) {
  let visible = initialVisible;
  return {
    isVisible: () => visible,
    toggle: vi.fn(async () => { visible = !visible; }),
    hide: vi.fn(async () => { visible = false; }),
    updatePreview: vi.fn(),
  };
}

function keyEvent(key, target, extra = {}) {
  return {
    key,
    target,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    preventDefault: vi.fn(),
    ...extra,
  };
}

describe("主窗口键盘路由", () => {
  let list;
  let preview;
  let codec;
  let hidePanel;
  let pinClip;

  function router(previewPanel = preview) {
    return createKeyboardRouter({
      clipboardList: list,
      previewPanel,
      codec,
      pinClip,
      hidePanel,
    });
  }

  beforeEach(() => {
    document.body.innerHTML = `
      <div id="clipboard-react-root"></div>
      <div id="preview-panel">
        <div id="translation-react-root">
          <button id="translation-action-react">Translate</button>
        </div>
      </div>
    `;
    list = fakeList();
    preview = fakePreview();
    codec = { toggle: vi.fn(), isVisible: () => false };
    hidePanel = vi.fn();
    pinClip = vi.fn().mockResolvedValue("pinned");
  });

  it("Tab 打开预览并把焦点交给翻译面板", () => {
    router().onKeyDown(keyEvent("Tab", document.body));
    expect(preview.toggle).toHaveBeenCalled();
    expect(preview.updatePreview).toHaveBeenCalledWith({ id: 7 });
    expect(document.activeElement.id).toBe("translation-action-react");
  });

  it("焦点在翻译面板里时 Tab 关掉预览并撤回焦点，不再是键盘死区", () => {
    const button = document.getElementById("translation-action-react");
    button.focus();
    preview = fakePreview(true);
    router().onKeyDown(keyEvent("Tab", button));
    expect(preview.hide).toHaveBeenCalled();
    expect(document.activeElement).not.toBe(button);
    expect(hidePanel).not.toHaveBeenCalled();
  });

  it("焦点在翻译面板里时方向键仍归面板自己（保留原生滚动语义）", () => {
    const button = document.getElementById("translation-action-react");
    const event = keyEvent("ArrowDown", button);
    router().onKeyDown(event);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(list.moveRow).not.toHaveBeenCalled();
  });

  it("预览打开时第一次 Esc 只关预览，第二次才隐藏窗口", () => {
    preview = fakePreview(true);
    const r = router();
    r.onKeyDown(keyEvent("Escape", document.body));
    expect(preview.hide).toHaveBeenCalledTimes(1);
    expect(hidePanel).not.toHaveBeenCalled();

    r.onKeyDown(keyEvent("Escape", document.body));
    expect(hidePanel).toHaveBeenCalledTimes(1);
  });

  it("焦点在翻译面板里的 Esc 关预览而不牵连列表展开状态", () => {
    const button = document.getElementById("translation-action-react");
    button.focus();
    list = fakeList({ expanded: true });
    preview = fakePreview(true);
    router().onKeyDown(keyEvent("Escape", button));
    expect(preview.hide).toHaveBeenCalled();
    expect(list.collapseActions).not.toHaveBeenCalled();
    expect(hidePanel).not.toHaveBeenCalled();
  });

  it("列表侧的 Esc 先收起行按钮组", () => {
    list = fakeList({ expanded: true });
    router().onKeyDown(keyEvent("Escape", document.body));
    expect(list.collapseActions).toHaveBeenCalled();
    expect(preview.hide).not.toHaveBeenCalled();
    expect(hidePanel).not.toHaveBeenCalled();
  });

  it("方向键与数字键仍走列表", async () => {
    const r = router();
    r.onKeyDown(keyEvent("ArrowDown", document.body));
    expect(list.moveRow).toHaveBeenCalledWith(1);
    r.onKeyDown(keyEvent("w", document.body));
    expect(list.moveRow).toHaveBeenCalledWith(-1);
    r.onKeyDown(keyEvent("3", document.body));
    expect(list.selectByIndex).toHaveBeenCalledWith(2);
    await vi.waitFor(() => expect(hidePanel).toHaveBeenCalled());
  });

  it("Ctrl+P pin 当前条目，反引号切换编解码面板", () => {
    const r = router();
    r.onKeyDown(keyEvent("p", document.body, { ctrlKey: true }));
    expect(pinClip).toHaveBeenCalledWith(7);
    r.onKeyDown(keyEvent("`", document.body));
    expect(codec.toggle).toHaveBeenCalled();
  });
});
