/**
 * app.js — 主窗口入口 + 键盘路由
 */

import * as theme         from "./theme.js";
import * as clipboardList from "./clipboard-list.js";
import * as searchBar     from "./search-bar.js";
import * as segmentTabs   from "./segment-tabs.js";
import * as i18n          from "../i18n/i18n.js";
import {
  getConfig, onClipAdded, onClipRemoved, onConfigChanged,
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
  });

  await clipboardList.refresh();

  await onClipAdded((clip) => clipboardList.prependClip(clip));
  await onClipRemoved((id) => clipboardList.removeClip(id));
  await onConfigChanged((newConfig) => {
    theme.applyTheme(newConfig.theme || "light");
    i18n.init(newConfig.language || "auto");
    searchBar.refreshLabels();
    segmentTabs.refreshLabels();
  });

  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("focus", onWindowFocus);
});

function onKeyDown(e) {
  // 搜索条聚焦时：不拦截普通字符；只接管 Esc / Enter
  if (searchBar.isVisible() && document.activeElement?.classList.contains("search-bar-input")) {
    if (e.key === "Escape") {
      e.preventDefault();
      const stage = searchBar.dismissStage();
      if (stage === "panel") tryHidePanel();
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
      // 优先：收回按钮组 → 收起搜索条 → 隐藏面板
      if (clipboardList.hasExpanded()) {
        clipboardList.collapseActions();
        return;
      }
      if (searchBar.isVisible()) {
        const stage = searchBar.dismissStage();
        if (stage === "panel") tryHidePanel();
        return;
      }
      tryHidePanel();
      return;
  }
}

function tryHidePanel() {
  // 失焦逻辑由 Tauri WindowEvent 处理；这里发一次 blur 触发它
  if (window.__TAURI__) {
    window.__TAURI__.window?.getCurrentWindow?.()?.hide?.();
  }
}

async function onWindowFocus() {
  await clipboardList.refresh();
}
