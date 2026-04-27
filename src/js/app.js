/**
 * app.js — 主窗口入口 + 键盘路由
 */

import * as theme         from "./theme.js";
import * as clipboardList from "./clipboard-list.js";
import * as searchBar     from "./search-bar.js";
import * as segmentTabs   from "./segment-tabs.js";
import * as i18n          from "../i18n/i18n.js";
import { initUpdateModal, checkForUpdate } from "./update-modal.js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  getConfig, onClipAdded, onClipRemoved, onConfigChanged,
  onShortcutRegisterFailed,
} from "./api.js";
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

  clipboardList.init({
    listEl,
    emptyEl,
    onCountsChange: (counts) => segmentTabs.setCounts(counts),
    onSummonSearch: (source) => searchBar.summon(source),
    onModeChange: (mode) => segmentTabs.setMode(mode),
  });
  clipboardList.setDeleteConfirmMs(config.delete_confirm_ms || 1200);

  await clipboardList.refresh();

  await onClipAdded((clip) => {
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
  });

  await onShortcutRegisterFailed((shortcut) => {
    console.warn(`快捷键 "${shortcut}" 注册失败，请在设置中更换快捷键`);
  });

  // 初始化更新弹窗并自动检查
  initUpdateModal();
  checkForUpdate(false).catch(console.warn);

  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("focus", onWindowFocus);
  window.addEventListener("blur", onWindowBlur);
});

function onKeyDown(e) {
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

  switch (e.key) {
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
      clipboardList.moveCol(-1);
      return;
    case "ArrowRight":
    case "d":
    case "D":
      e.preventDefault();
      // 行体上：favorites mode → 切回 all；all mode → 展开按钮组
      // 按钮区：按钮间右移
      if (clipboardList.canExpandHere()) {
        if (clipboardList.getPanelMode() === "favorites") {
          clipboardList.setPanelMode("all");
        } else {
          clipboardList.expandRowActions();
        }
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
  }
}

function tryHidePanel() {
  getCurrentWindow().hide();
}

async function onWindowFocus() {
  // 仅在有新数据时才全量刷新，否则只恢复渲染
  if (clipboardList.isDirty()) {
    await clipboardList.refresh();
  } else {
    clipboardList.restoreRender();
  }
}

function onWindowBlur() {
  clipboardList.releaseMemory();
}
