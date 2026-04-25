/**
 * app.js — 应用入口
 */

import * as theme         from "./theme.js";
import * as clipboardList from "./clipboard-list.js";
import * as search        from "./search.js";
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
  // 加载配置
  let config;
  try {
    config = await getConfig();
  } catch (err) {
    console.warn("配置加载失败:", err);
    config = { theme: "light", language: "auto" };
  }

  theme.applyTheme(config.theme || "light");
  i18n.init(config.language || "auto");

  const clipListEl   = document.getElementById("clip-list");
  const emptyStateEl = document.getElementById("empty-state");
  clipboardList.init(clipListEl, emptyStateEl);
  await clipboardList.refresh();

  const searchInput = document.getElementById("search-input");
  search.init(searchInput, (query) => clipboardList.setQuery(query));

  // 标签页
  document.querySelectorAll(".tab-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll(".tab-btn").forEach((b) => {
        b.classList.remove("active");
        b.setAttribute("aria-selected", "false");
      });
      btn.classList.add("active");
      btn.setAttribute("aria-selected", "true");
      clipboardList.setFavoritesOnly(btn.dataset.tab === "favorites");
    });
  });

  // 事件
  await onClipAdded((clip) => clipboardList.prependClip(clip));
  await onClipRemoved((id) => clipboardList.removeClip(id));
  await onConfigChanged((newConfig) => {
    theme.applyTheme(newConfig.theme || "light");
    i18n.init(newConfig.language || "auto");
  });

  // 键盘
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("focus", onWindowFocus);
});

function onKeyDown(e) {
  switch (e.key) {
    case "Escape":      clipboardList.closeOpenMenu(); break;
    case "ArrowUp":     e.preventDefault(); clipboardList.moveSelection(-1); break;
    case "ArrowDown":   e.preventDefault(); clipboardList.moveSelection(1); break;
    case "Enter":       e.preventDefault(); clipboardList.confirmSelection(); break;
  }
}

async function onWindowFocus() {
  await clipboardList.refresh();
  search.focus();
}
