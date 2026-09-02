import { describe, it, expect, beforeEach, vi } from "vitest";
import { createKeyboardRouter, resolveKeyboardMode } from "../js/keyboard-router.js";

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

function fakeCodec(initialVisible = false) {
  let visible = initialVisible;
  return {
    isVisible: () => visible,
    toggle: vi.fn(async () => { visible = !visible; }),
    hide: vi.fn(async () => { visible = false; }),
  };
}

function keyEvent(key, target, extra = {}) {
  return {
    key,
    target,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    preventDefault: vi.fn(),
    ...extra,
  };
}

describe("键盘归属状态机", () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <div id="codec-panel"><textarea id="codec-input"></textarea></div>
      <div id="list-panel" tabindex="-1"><input class="search-bar-input" /></div>
      <div id="translation-react-root"><button id="translation-action-react">T</button></div>
    `;
  });

  it("按焦点位置解析，codec > search > translation > list", () => {
    const codecInput = document.getElementById("codec-input");
    const translateButton = document.getElementById("translation-action-react");
    expect(resolveKeyboardMode({ target: codecInput })).toBe("codec");
    // 侧栏拥有焦点时优先级最高，即使搜索框也是"可见且聚焦"的状态
    expect(resolveKeyboardMode({ target: codecInput }, { searchFocused: true })).toBe("codec");
    expect(resolveKeyboardMode({ target: document.body }, { searchFocused: true })).toBe("search");
    expect(resolveKeyboardMode({ target: translateButton })).toBe("translation");
    expect(resolveKeyboardMode({ target: document.body })).toBe("list");
    // target 可能是 document/window，没有 closest 也不能崩
    expect(resolveKeyboardMode({ target: document })).toBe("list");
    expect(resolveKeyboardMode(undefined)).toBe("list");
  });
});

describe("主窗口键盘路由", () => {
  let list;
  let preview;
  let codec;
  let hidePanel;
  let pinClip;
  let translation;
  let openImage;

  function router(previewPanel = preview) {
    return createKeyboardRouter({
      clipboardList: list,
      previewPanel,
      codec,
      pinClip,
      hidePanel,
      translation,
      openImage,
    });
  }

  beforeEach(() => {
    document.body.innerHTML = `
      <div id="codec-panel">
        <textarea id="codec-input"></textarea>
      </div>
      <div id="list-panel" tabindex="-1">
        <div id="clipboard-react-root"></div>
        <input class="search-bar-input" />
      </div>
      <div id="preview-panel">
        <div id="translation-react-root">
          <button id="translation-action-react">Translate</button>
        </div>
      </div>
    `;
    list = fakeList();
    preview = fakePreview();
    codec = fakeCodec();
    hidePanel = vi.fn();
    pinClip = vi.fn().mockResolvedValue("pinned");
    translation = { translate: vi.fn().mockResolvedValue(undefined) };
    openImage = vi.fn().mockResolvedValue("pin-file-1");
  });

  describe("列表模式", () => {
    it("Ctrl/Cmd+O 从任意焦点打开图片选择器", () => {
      const listEvent = keyEvent("o", document.body, { ctrlKey: true });
      router().onKeyDown(listEvent);
      expect(listEvent.preventDefault).toHaveBeenCalled();
      expect(openImage).toHaveBeenCalledTimes(1);

      const codecEvent = keyEvent("O", document.getElementById("codec-input"), { metaKey: true });
      router().onKeyDown(codecEvent);
      expect(codecEvent.preventDefault).toHaveBeenCalled();
      expect(openImage).toHaveBeenCalledTimes(2);
    });
    it("Tab 只切预览开合，焦点留在列表", () => {
      router().onKeyDown(keyEvent("Tab", document.body));
      expect(preview.toggle).toHaveBeenCalled();
      expect(preview.updatePreview).toHaveBeenCalledWith({ id: 7 });
      // 焦点绝不能进翻译面板，否则上下键会变成翻动翻译区
      expect(document.activeElement.id).toBe("list-panel");
    });

    it("预览打开后方向键与 ws 仍然移动列表选择", () => {
      preview = fakePreview(true);
      const r = router();
      r.onKeyDown(keyEvent("ArrowDown", document.body));
      r.onKeyDown(keyEvent("s", document.body));
      expect(list.moveRow).toHaveBeenNthCalledWith(1, 1);
      expect(list.moveRow).toHaveBeenNthCalledWith(2, 1);
      r.onKeyDown(keyEvent("ArrowUp", document.body));
      r.onKeyDown(keyEvent("w", document.body));
      expect(list.moveRow).toHaveBeenNthCalledWith(3, -1);
      expect(list.moveRow).toHaveBeenNthCalledWith(4, -1);
    });

    it("预览打开时 Tab 关预览，Shift+Tab 才把焦点送进翻译面板", () => {
      preview = fakePreview(true);
      const r = router();
      r.onKeyDown(keyEvent("Tab", document.body, { shiftKey: true }));
      expect(document.activeElement.id).toBe("translation-action-react");
      expect(preview.hide).not.toHaveBeenCalled();

      r.onKeyDown(keyEvent("Tab", document.body));
      expect(preview.hide).toHaveBeenCalled();
    });

    it("预览关着时 Shift+Tab 不做任何事", () => {
      const event = keyEvent("Tab", document.body, { shiftKey: true });
      router().onKeyDown(event);
      expect(event.preventDefault).not.toHaveBeenCalled();
      expect(document.activeElement.id).not.toBe("translation-action-react");
    });

    // 翻译进行中 Translate 按钮是 disabled 的，聚焦不上去；这时候如果照样
    // preventDefault，Shift+Tab 就变成"按了完全没反应"，键还被吞了。
    it("翻译按钮被禁用时退到面板里其它可聚焦元素", () => {
      preview = fakePreview(true);
      document.getElementById("translation-action-react").disabled = true;
      const copy = document.createElement("button");
      copy.id = "translation-card-copy";
      document.getElementById("translation-react-root").appendChild(copy);

      const event = keyEvent("Tab", document.body, { shiftKey: true });
      router().onKeyDown(event);
      expect(document.activeElement.id).toBe("translation-card-copy");
      expect(event.preventDefault).toHaveBeenCalled();
    });

    it("面板里一个能聚焦的元素都没有时不吞掉 Shift+Tab", () => {
      preview = fakePreview(true);
      // 没有条目时 TranslationPanel 直接 return null，挂载点是空的
      document.getElementById("translation-react-root").replaceChildren();

      const event = keyEvent("Tab", document.body, { shiftKey: true });
      router().onKeyDown(event);
      expect(event.preventDefault).not.toHaveBeenCalled();
    });

    it("Ctrl+Enter 在预览打开时翻译当前条目，关着时不触发也不吞按键", () => {
      const closed = keyEvent("Enter", document.body, { ctrlKey: true });
      router().onKeyDown(closed);
      expect(translation.translate).not.toHaveBeenCalled();
      expect(closed.preventDefault).not.toHaveBeenCalled();
      expect(list.activateFocus).not.toHaveBeenCalled();

      preview = fakePreview(true);
      router().onKeyDown(keyEvent("Enter", document.body, { ctrlKey: true }));
      expect(translation.translate).toHaveBeenCalled();
      expect(list.activateFocus).not.toHaveBeenCalled();
    });

    it("带修饰键的普通字母不会被当成列表导航", () => {
      const r = router();
      r.onKeyDown(keyEvent("s", document.body, { ctrlKey: true }));
      r.onKeyDown(keyEvent("d", document.body, { altKey: true }));
      expect(list.moveRow).not.toHaveBeenCalled();
      expect(list.moveCol).not.toHaveBeenCalled();
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

  describe("编解码面板模式", () => {
    let input;

    beforeEach(() => {
      input = document.getElementById("codec-input");
      codec = fakeCodec(true);
      input.focus();
    });

    it("主栏按键全部交给面板，不驱动列表也不被吞掉", () => {
      const r = router();
      for (const key of ["w", "a", "s", "d", "W", "S", "1", "0", "Enter", " ", "ArrowDown", "ArrowLeft"]) {
        const event = keyEvent(key, input);
        r.onKeyDown(event);
        expect(event.preventDefault, `${key} 不应被拦截`).not.toHaveBeenCalled();
      }
      expect(list.moveRow).not.toHaveBeenCalled();
      expect(list.moveCol).not.toHaveBeenCalled();
      expect(list.selectByIndex).not.toHaveBeenCalled();
      expect(list.activateFocus).not.toHaveBeenCalled();
      expect(list.expandRowActions).not.toHaveBeenCalled();
      expect(hidePanel).not.toHaveBeenCalled();
    });

    it("反引号关面板并把焦点收回列表", () => {
      const event = keyEvent("`", input);
      router().onKeyDown(event);
      expect(event.preventDefault).toHaveBeenCalled();
      expect(codec.hide).toHaveBeenCalled();
      expect(codec.toggle).not.toHaveBeenCalled();
      expect(document.activeElement.id).toBe("list-panel");
    });

    it("Esc 也关面板，不会直接隐藏窗口丢掉输入内容", () => {
      router().onKeyDown(keyEvent("Escape", input));
      expect(codec.hide).toHaveBeenCalled();
      expect(hidePanel).not.toHaveBeenCalled();
      expect(preview.hide).not.toHaveBeenCalled();
    });
  });

  describe("翻译面板模式", () => {
    let button;

    beforeEach(() => {
      button = document.getElementById("translation-action-react");
      button.focus();
      preview = fakePreview(true);
    });

    it("方向键仍归面板自己（保留原生滚动语义）", () => {
      const event = keyEvent("ArrowDown", button);
      router().onKeyDown(event);
      expect(event.preventDefault).not.toHaveBeenCalled();
      expect(list.moveRow).not.toHaveBeenCalled();
    });

    it("Esc/Tab 只把焦点交回列表，预览留着", () => {
      const r = router();
      r.onKeyDown(keyEvent("Escape", button));
      expect(document.activeElement.id).toBe("list-panel");
      expect(preview.hide).not.toHaveBeenCalled();
      expect(hidePanel).not.toHaveBeenCalled();

      button.focus();
      r.onKeyDown(keyEvent("Tab", button));
      expect(document.activeElement.id).toBe("list-panel");
      expect(preview.hide).not.toHaveBeenCalled();
    });

    it("焦点回到列表后再按 Esc 才关预览", () => {
      const r = router();
      r.onKeyDown(keyEvent("Escape", button));
      r.onKeyDown(keyEvent("Escape", document.getElementById("list-panel")));
      expect(preview.hide).toHaveBeenCalledTimes(1);
      expect(hidePanel).not.toHaveBeenCalled();
    });

    it("Ctrl+Enter 触发翻译，反引号仍能打开编解码面板", () => {
      const r = router();
      r.onKeyDown(keyEvent("Enter", button, { ctrlKey: true }));
      expect(translation.translate).toHaveBeenCalled();
      r.onKeyDown(keyEvent("`", button));
      expect(codec.toggle).toHaveBeenCalled();
    });
  });

  describe("搜索模式", () => {
    it("普通字符交给输入框，只接管 Esc", () => {
      list = fakeList({ searchVisible: true });
      const input = document.querySelector(".search-bar-input");
      input.focus();
      const r = router();

      // 反引号在搜索框里是普通字符，不能被拿去开侧栏
      const backtick = keyEvent("`", input);
      r.onKeyDown(backtick);
      expect(backtick.preventDefault).not.toHaveBeenCalled();
      expect(codec.toggle).not.toHaveBeenCalled();

      const letter = keyEvent("s", input);
      r.onKeyDown(letter);
      expect(letter.preventDefault).not.toHaveBeenCalled();
      expect(list.moveRow).not.toHaveBeenCalled();

      r.onKeyDown(keyEvent("Escape", input));
      expect(list.search.dismissStage).toHaveBeenCalled();
      expect(hidePanel).toHaveBeenCalled();
    });
  });
});
