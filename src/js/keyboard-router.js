/**
 * keyboard-router.js — 主窗口键盘路由
 *
 * 从 app.js 抽出，依赖以工厂参数注入，便于在 jsdom 下单测键位语义
 * （app.js 本身在模块顶层就要连 IPC，测不动）。
 */

/**
 * @param {object} deps
 * @param {object} deps.clipboardList 列表 facade（moveRow/moveCol/search/...）
 * @param {object} deps.previewPanel  预览面板（isVisible/toggle/hide/updatePreview）
 * @param {object} deps.codec         编解码面板（toggle）
 * @param {function} deps.pinClip     Pin 当前条目
 * @param {function} deps.hidePanel   隐藏主窗口
 */
export function createKeyboardRouter({
  clipboardList,
  previewPanel,
  codec,
  pinClip,
  hidePanel,
}) {
  /** 关预览：把焦点从翻译面板里的按钮撤回来，否则下一次按键仍会落在死区分支 */
  function closePreview() {
    const active = document.activeElement;
    if (active?.closest?.("#translation-react-root")) {
      active.blur();
    }
    void previewPanel.hide();
  }

  function focusTranslationPanel() {
    const target = document.getElementById("translation-sensitive-react")
      || document.getElementById("translation-action-react");
    target?.focus();
  }

  function onKeyDown(e) {
    const inTranslationPanel = Boolean(e.target?.closest?.("#translation-react-root"));

    // 翻译区使用原生按钮和可滚动结果，保留其键盘语义；
    // 但 Tab/Esc 必须交给全局路由，否则焦点一旦进入面板就再也回不到列表。
    if (inTranslationPanel && e.key !== "Escape" && e.key !== "Tab") {
      return;
    }

    // 搜索条聚焦时：不拦截普通字符；只接管 Esc / Enter
    if (clipboardList.search.isVisible()
      && document.activeElement?.classList.contains("search-bar-input")) {
      if (e.key === "Escape") {
        e.preventDefault();
        const stage = clipboardList.search.dismissStage();
        if (stage === "panel") {
          clipboardList.hasExpanded() ? clipboardList.collapseActions() : hidePanel();
        }
      }
      return; // 其它键交给 input
    }

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
          const stage = clipboardList.search.dismissStage();
          if (stage === "panel") {
            clipboardList.hasExpanded() ? clipboardList.collapseActions() : hidePanel();
          }
        } else if (inTranslationPanel && previewPanel.isVisible()) {
          // 焦点在翻译面板里：Esc 只关预览，不牵连列表的展开状态
          closePreview();
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
        if (previewPanel.isVisible()) {
          closePreview();
        } else {
          void previewPanel.toggle();
          previewPanel.updatePreview(clipboardList.getFocusedClip());
          focusTranslationPanel();
        }
        return;
      case "`":
        e.preventDefault();
        codec.toggle();
        return;
    }
  }

  return { onKeyDown };
}
