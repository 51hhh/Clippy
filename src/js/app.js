/**
 * app.js — 主窗口入口 + 键盘路由
 */

import * as theme         from "./theme.js";
import * as clipboardList from "./clipboard-list.js";
import * as searchBar     from "./search-bar.js";
import * as segmentTabs   from "./segment-tabs.js";
import * as previewPanel  from "./preview-panel.js";
import * as translationPanel from "./translation-panel.js";
import * as codec         from "./codec.js";
import * as i18n          from "../i18n/i18n.js";
import { initUpdateModal, checkForUpdate } from "./update-modal.js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  getConfig, getClips, onClipAdded, onClipRemoved, onConfigChanged,
  onShortcutRegisterFailed, onPinCurrent, pinClip,
} from "./api.ts";
import "../styles/themes.css";
import "../styles/base.css";
import "../styles/components.css";

function whenReady(fn) {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", fn);
  } else {
    fn();
  }
}

whenReady(async () => {
  let config;
  try {
    config = await getConfig();
  } catch (err) {
    console.warn("配置加载失败:", err);
    config = { theme: "light", language: "auto" };
  }

  theme.applyTheme(config.theme || "light");
  i18n.init(config.language || "auto");

  const listEl    = document.getElementById("clip-list");
  const emptyEl   = document.getElementById("empty-state");
  const searchEl  = document.getElementById("search-bar");
  const segmentEl = document.getElementById("segment-tabs");

  segmentTabs.init(segmentEl, (mode) => clipboardList.setPanelMode(mode));
  searchBar.init(searchEl, (q) => clipboardList.setQuery(q));
  previewPanel.init();
  translationPanel.init(config);
  codec.init();

  clipboardList.init({
    listEl,
    emptyEl,
    onCountsChange: (counts) => segmentTabs.setCounts(counts),
    onSummonSearch: (source) => searchBar.summon(source),
    onModeChange: (mode) => segmentTabs.setMode(mode),
    onFocusChange: (clip) => {
      previewPanel.updatePreview(clip);
      translationPanel.updateClip(clip);
    },
  });

  await clipboardList.refresh();

  await onClipAdded((clip) => {
    console.debug("[clip-added]", clip.id, clip.content_type, clip.byte_size);
    clipboardList.markDirty();
    clipboardList.prependClip(clip);
  });
  await onClipRemoved((id) => {
    clipboardList.markDirty();
    clipboardList.removeClip(id);
  });
  await onConfigChanged((newConfig) => {
    theme.applyTheme(newConfig.theme || "light");
    i18n.init(newConfig.language || "auto");
    searchBar.refreshLabels();
    segmentTabs.refreshLabels();
    translationPanel.updateConfig(newConfig);
  });

  await onShortcutRegisterFailed((shortcut) => {
    console.warn(`快捷键 "${shortcut}" 注册失败，请在设置中更换快捷键`);
  });

  await onPinCurrent(async () => {
    // 系统快捷键触发：pin 焦点条目，若列表空则从后端获取最新条目
    let clip = clipboardList.getFocusedClip() || clipboardList.getLatestClip();
    if (!clip) {
      const clips = await getClips(null, false, 0, 1);
      clip = clips[0] || null;
    }
    if (clip) pinClip(clip.id).catch(err => console.warn("Pin 失败:", err));
  });

  // 初始化更新弹窗并自动检查
  initUpdateModal();
  checkForUpdate(false).catch(console.warn);

  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("focus", onWindowFocus);
  window.addEventListener("blur", onWindowBlur);
});

function onKeyDown(e) {
  // 翻译区使用原生按钮和可滚动结果，保留其键盘语义；Esc 仍交给全局关闭逻辑。
  if (e.target?.closest?.("#translation-panel") && e.key !== "Escape") {
    return;
  }

  // 搜索条聚焦时：不拦截普通字符；只接管 Esc / Enter
  if (searchBar.isVisible() && document.activeElement?.classList.contains("search-bar-input")) {
    if (e.key === "Escape") {
      e.preventDefault();
      const stage = searchBar.dismissStage();
      if (stage === "panel") {
        clipboardList.hasExpanded() ? clipboardList.collapseActions() : tryHidePanel();
      }
      return;
    }
    return; // 其它键交给 input
  }

  // Ctrl+P：Pin 当前焦点条目到桌面
  if (e.ctrlKey && !e.shiftKey && !e.altKey && (e.key === "p" || e.key === "P")) {
    e.preventDefault();
    const clip = clipboardList.getFocusedClip();
    if (clip) {
      pinClip(clip.id).then(label => console.log("Pin 成功:", label))
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
      clipboardList.selectByIndex(idx).then(ok => {
        if (ok) getCurrentWindow().hide();
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
      if (searchBar.isVisible()) {
        const stage = searchBar.dismissStage();
        if (stage === "panel") {
          clipboardList.hasExpanded() ? clipboardList.collapseActions() : tryHidePanel();
        }
      } else if (clipboardList.hasExpanded()) {
        clipboardList.collapseActions();
      } else {
        tryHidePanel();
      }
      return;
    case "Tab":
      e.preventDefault();
      if (!previewPanel.isVisible()) {
        previewPanel.toggle();
        previewPanel.updatePreview(clipboardList.getFocusedClip());
        translationPanel.focusAction();
      } else {
        previewPanel.toggle();
      }
      return;
    case "`":
      e.preventDefault();
      codec.toggle();
      return;
  }
}

function tryHidePanel() {
  getCurrentWindow().hide();
}

async function onWindowFocus() {
  console.debug("[focus] dirty=", clipboardList.isDirty());
  // 仅在有新数据时才全量刷新，否则只恢复渲染
  if (clipboardList.isDirty()) {
    await clipboardList.refresh();
  } else {
    clipboardList.restoreRender();
  }
}

function onWindowBlur() {
  if (previewPanel.isVisible() || codec.isVisible()) return; // 面板打开时不隐藏窗口
  clipboardList.releaseMemory();
  previewPanel.clearContent();
  translationPanel.clear();
}
