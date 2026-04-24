/**
 * app.js — 应用入口
 * 负责模块初始化、事件总线连接、键盘导航和窗口焦点处理。
 */

import * as theme          from "./theme.js";
import * as clipboardList  from "./clipboard-list.js";
import * as search         from "./search.js";
import { onClipAdded, onClipRemoved } from "./api.js";

// ── DOMContentLoaded ───────────────────────────────────────────────────────
window.addEventListener("DOMContentLoaded", async () => {
  // ── 1. 主题 ──
  await theme.init();

  // ── 2. 剪贴板列表 ──
  const clipListEl   = document.getElementById("clip-list");
  const emptyStateEl = document.getElementById("empty-state");
  clipboardList.init(clipListEl, emptyStateEl);
  await clipboardList.refresh();

  // ── 3. 搜索框 ──
  const searchInput = document.getElementById("search-input");
  search.init(searchInput, (query) => {
    clipboardList.setQuery(query);
  });

  // ── 4. 标签页 ──
  const tabBtns = document.querySelectorAll(".tab-btn");
  tabBtns.forEach((btn) => {
    btn.addEventListener("click", () => {
      // 更新按钮激活状态
      tabBtns.forEach((b) => {
        b.classList.remove("active");
        b.setAttribute("aria-selected", "false");
      });
      btn.classList.add("active");
      btn.setAttribute("aria-selected", "true");

      const favOnly = btn.dataset.tab === "favorites";
      clipboardList.setFavoritesOnly(favOnly);
    });
  });

  // ── 5. 后端事件监听 ──
  await onClipAdded((clip) => {
    clipboardList.prependClip(clip);
  });

  await onClipRemoved((id) => {
    clipboardList.removeClip(id);
  });

  // ── 6. 键盘导航 ──
  window.addEventListener("keydown", _onKeyDown);

  // ── 7. 窗口获得焦点时刷新列表并聚焦搜索框 ──
  window.addEventListener("focus", _onWindowFocus);
});

// ── 键盘处理 ───────────────────────────────────────────────────────────────

function _onKeyDown(e) {
  switch (e.key) {
    case "Escape":
      // 先尝试关闭操作菜单，再关闭窗口
      clipboardList.closeOpenMenu();
      break;

    case "ArrowUp":
      e.preventDefault();
      clipboardList.moveSelection(-1);
      break;

    case "ArrowDown":
      e.preventDefault();
      clipboardList.moveSelection(1);
      break;

    case "Enter":
      e.preventDefault();
      clipboardList.confirmSelection();
      break;

    default:
      break;
  }
}

// ── 窗口焦点 ───────────────────────────────────────────────────────────────

async function _onWindowFocus() {
  await clipboardList.refresh();
  search.focus();
}
