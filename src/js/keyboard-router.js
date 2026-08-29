/**
 * keyboard-router.js — 主窗口键盘路由
 *
 * 从 app.js 抽出，依赖以工厂参数注入，便于在 jsdom 下单测键位语义
 * （app.js 本身在模块顶层就要连 IPC，测不动）。
 *
 * 键盘归属是一个"按焦点位置解析"的状态机：同一时刻只有一个区域拥有键盘，
 * 由 `resolveKeyboardMode` 单点判定，各模式的按键契约见 docs/architecture.md。
 * 之所以按焦点而不是按"面板是否可见"判定：键盘操作下焦点不会自己跑出侧栏，
 * 因此"只有 ` 能切回列表"依然成立，而鼠标点回中间列表能立刻把键盘交还列表。
 */

const CODEC_PANEL_SELECTOR = "#codec-panel";
const TRANSLATION_ROOT_SELECTOR = "#translation-react-root";

/** 焦点是否落在某个容器内（target 可能是 document/window，没有 closest） */
function inside(target, selector) {
  return Boolean(target?.closest?.(selector));
}

/**
 * 解析这次按键归谁。先匹配先赢：codec > search > translation > list。
 *
 * @param {KeyboardEvent|{target: EventTarget}} event
 * @param {{searchFocused?: boolean}} context
 * @returns {"codec"|"search"|"translation"|"list"}
 */
export function resolveKeyboardMode(event, { searchFocused = false } = {}) {
  const target = event?.target;
  if (inside(target, CODEC_PANEL_SELECTOR)) return "codec";
  if (searchFocused) return "search";
  if (inside(target, TRANSLATION_ROOT_SELECTOR)) return "translation";
  return "list";
}

/**
 * @param {object} deps
 * @param {object} deps.clipboardList 列表 facade（moveRow/moveCol/search/...）
 * @param {object} deps.previewPanel  预览面板（isVisible/toggle/hide/updatePreview）
 * @param {object} deps.codec         编解码面板（toggle/hide/isVisible）
 * @param {function} deps.pinClip     Pin 当前条目
 * @param {function} deps.hidePanel   隐藏主窗口
 * @param {object} [deps.translation] 翻译面板动作（translate），默认空实现
 */
export function createKeyboardRouter({
  clipboardList,
  previewPanel,
  codec,
  pinClip,
  hidePanel,
  translation = {},
}) {
  /** 把焦点收回中间列表，避免停在 body 这种"谁也不拥有"的中间态 */
  function focusList() {
    document.getElementById("list-panel")?.focus();
  }

  /** 关预览：把焦点从翻译面板里的按钮撤回来，否则下一次按键仍会落在翻译分支 */
  function closePreview() {
    returnFocusToList();
    void previewPanel.hide();
  }

  function returnFocusToList() {
    const active = document.activeElement;
    if (active?.closest?.(TRANSLATION_ROOT_SELECTOR)) {
      active.blur();
    }
    focusList();
  }

  function focusTranslationPanel() {
    const target = document.getElementById("translation-sensitive-react")
      || document.getElementById("translation-action-react");
    target?.focus();
  }

  function closeCodec() {
    Promise.resolve(codec.hide()).catch((error) => console.warn("codec hide:", error));
    focusList();
  }

  function dismissSearch() {
    const stage = clipboardList.search.dismissStage();
    if (stage === "panel") {
      clipboardList.hasExpanded() ? clipboardList.collapseActions() : hidePanel();
    }
  }

  // ── 左侧编解码面板拥有键盘：只有 ` 和 Esc 会被拦下来关面板，其余全部交给面板自己 ──
  function onCodecKeyDown(e) {
    if (e.key !== "`" && e.key !== "Escape") return;
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    e.preventDefault();
    closeCodec();
  }

  // ── 搜索输入框拥有键盘：只接管 Esc ──
  function onSearchKeyDown(e) {
    if (e.key !== "Escape") return; // 其它键交给 input
    e.preventDefault();
    dismissSearch();
  }

  // ── 翻译面板拥有键盘：保留原生滚动/按钮语义，只留回列表与触发翻译的出口 ──
  function onTranslationKeyDown(e) {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      void translation.translate?.();
      return;
    }
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    // Esc/Tab 只把焦点交回列表，预览留着；再按一次 Esc/Tab 才由列表分支关预览。
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      returnFocusToList();
      return;
    }
    if (e.key === "`") {
      e.preventDefault();
      void codec.toggle();
    }
  }

  // ── 中间列表拥有键盘（预览开着也一样，方向键始终归列表） ──
  function onListKeyDown(e) {
    // Ctrl+P：Pin 当前焦点条目到桌面
    if (e.ctrlKey && !e.shiftKey && !e.altKey && (e.key === "p" || e.key === "P")) {
      e.preventDefault();
      const clip = clipboardList.getFocusedClip();
      if (clip) {
        Promise.resolve(pinClip(clip.id))
          .then(label => console.log("Pin 成功:", label))
          .catch(err => console.warn("Pin 失败:", err));
      }
      return;
    }

    // Ctrl+Enter：翻译当前条目（预览关着时没有可展示结果的地方，不触发）
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      if (!previewPanel.isVisible()) return;
      e.preventDefault();
      void translation.translate?.();
      return;
    }

    // 其余带修饰键的组合一律放行（Ctrl+S 之类不该被当成列表导航的 s）
    if (e.ctrlKey || e.altKey || e.metaKey) return;

    // Shift+Tab：显式把焦点送进翻译面板（预览开着才有面板）
    if (e.key === "Tab" && e.shiftKey) {
      if (!previewPanel.isVisible()) return;
      e.preventDefault();
      focusTranslationPanel();
      return;
    }

    switch (e.key) {
      // 数字键 1-9/0：直选第 1-10 条并粘贴
      case "1": case "2": case "3": case "4": case "5":
      case "6": case "7": case "8": case "9": case "0": {
        e.preventDefault();
        const idx = e.key === "0" ? 9 : parseInt(e.key) - 1;
        Promise.resolve(clipboardList.selectByIndex(idx)).then(ok => {
          if (ok) hidePanel();
        });
        return;
      }
      case "ArrowUp":
      case "w":
      case "W":
        e.preventDefault();
        clipboardList.moveRow(-1);
        return;
      case "ArrowDown":
      case "s":
      case "S":
        e.preventDefault();
        clipboardList.moveRow(1);
        return;
      case "ArrowLeft":
      case "a":
      case "A":
        e.preventDefault();
        // 收藏模式行体上：展开按钮组（按钮在左侧）
        if (clipboardList.getPanelMode() === "favorites" && clipboardList.canExpandHere()) {
          clipboardList.expandRowActions();
        } else {
          clipboardList.moveCol(-1);
        }
        return;
      case "ArrowRight":
      case "d":
      case "D":
        e.preventDefault();
        // 全部模式行体上：展开按钮组
        if (clipboardList.getPanelMode() === "all" && clipboardList.canExpandHere()) {
          clipboardList.expandRowActions();
        } else {
          clipboardList.moveCol(1);
        }
        return;
      case "Enter":
      case " ":
        e.preventDefault();
        clipboardList.activateFocus("keyboard");
        return;
      case "Escape":
        e.preventDefault();
        if (clipboardList.search.isVisible()) {
          dismissSearch();
        } else if (clipboardList.hasExpanded()) {
          clipboardList.collapseActions();
        } else if (previewPanel.isVisible()) {
          // 预览开着时第一次 Esc 关预览，第二次才隐藏窗口
          closePreview();
        } else {
          hidePanel();
        }
        return;
      case "Tab":
        e.preventDefault();
        // 只切预览开合，焦点始终留在列表，方向键不会跑去翻动翻译区。
        if (previewPanel.isVisible()) {
          closePreview();
        } else {
          void previewPanel.toggle();
          previewPanel.updatePreview(clipboardList.getFocusedClip());
          focusList();
        }
        return;
      case "`":
        e.preventDefault();
        void codec.toggle();
        return;
    }
  }

  function onKeyDown(e) {
    const searchFocused = clipboardList.search.isVisible()
      && Boolean(document.activeElement?.classList?.contains("search-bar-input"));
    switch (resolveKeyboardMode(e, { searchFocused })) {
      case "codec":
        onCodecKeyDown(e);
        return;
      case "search":
        onSearchKeyDown(e);
        return;
      case "translation":
        onTranslationKeyDown(e);
        return;
      default:
        onListKeyDown(e);
    }
  }

  return { onKeyDown };
}
