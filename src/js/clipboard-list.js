/**
 * clipboard-list.js — 剪贴板列表模块
 */

import { getClips, deleteClip, toggleFavorite, selectClip } from "./api.js";
import { t } from "../i18n/i18n.js";
import * as telemetry from "./telemetry.js";

const PAGE_SIZE = 20;

let _parent       = null;
let _emptyStateEl = null;
let _query        = "";
let _favoritesOnly = false;
let _allClips     = [];
let _selectedIdx  = -1;
let _openMenuId   = null;

export function init(parent, emptyStateEl) {
  _parent       = parent;
  _emptyStateEl = emptyStateEl;
}

export async function refresh() {
  try {
    _allClips = await getClips(_query || null, _favoritesOnly, 0, 200);
    telemetry.emit("clip-list:refresh", {
      query: _query,
      favoritesOnly: _favoritesOnly,
      count: _allClips.length,
    });
  } catch (e) {
    console.error("Clipboard query failed:", e);
    _allClips = [];
    telemetry.emit("clip-list:refresh-error", { message: String(e) });
  }
  render();
}

export function setQuery(query) {
  _query = query;
  refresh();
}

export function setFavoritesOnly(fav) {
  _favoritesOnly = fav;
  refresh();
}

export function prependClip(clip) {
  _allClips.unshift(clip);
  render();
}

export function removeClip(id) {
  _allClips = _allClips.filter((c) => c.id !== id);
  render();
}

export function moveSelection(delta) {
  const items = visibleItems();
  if (items.length === 0) return;
  _selectedIdx = Math.max(0, Math.min(items.length - 1, _selectedIdx + delta));
  updateSelection();
}

export function confirmSelection() {
  const items = visibleItems();
  if (_selectedIdx >= 0 && _selectedIdx < items.length) {
    const clip = items[_selectedIdx];
    selectClipAction(clip);
  }
}

export function closeOpenMenu() {
  if (_openMenuId !== null) {
    const menu = document.getElementById(`menu-${_openMenuId}`);
    if (menu) menu.classList.add("hidden");
    _openMenuId = null;
  }
}

// ── 渲染 ──

function visibleItems() {
  return _allClips.filter((c) => !_favoritesOnly || c.is_favorite);
}

function render() {
  if (!_parent) return;
  const clips = visibleItems();

  if (clips.length === 0) {
    _parent.classList.add("hidden");
    _emptyStateEl?.classList.remove("hidden");
    telemetry.emit("clip-list:render", { count: 0, empty: true });
    return;
  }

  _parent.classList.remove("hidden");
  _emptyStateEl?.classList.add("hidden");

  // 简单重新渲染（优化：可改为增量 diff）
  _parent.replaceChildren();
  clips.forEach((clip, idx) => {
    _parent.appendChild(buildItem(clip, idx));
  });
  _selectedIdx = -1;
  telemetry.emit("clip-list:render", { count: clips.length, empty: false });
}

function buildItem(clip, idx) {
  const el = document.createElement("div");
  el.className = `clip-item${clip.is_favorite ? " favorite" : ""}`;
  el.dataset.idx = idx;
  el.dataset.id  = clip.id;

  const preview = (clip.text_content || "").slice(0, 120);
  const timeStr = formatRelativeTime(clip.created_at);

  const previewEl = document.createElement("span");
  previewEl.className = "clip-preview";
  previewEl.textContent = preview;

  const metaEl = document.createElement("span");
  metaEl.className = "clip-meta";
  metaEl.textContent = `${timeStr} · ${fmtSize(clip.byte_size)}`;

  const starEl = document.createElement("span");
  starEl.className = "clip-star";
  starEl.textContent = clip.is_favorite ? "★" : "☆";

  el.append(previewEl, metaEl, starEl);

  el.addEventListener("click", (e) => {
    if (e.target.classList.contains("clip-star")) {
      toggleFav(clip);
      return;
    }
    selectClipAction(clip);
  });

  // 右键菜单
  el.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    showMenu(clip, e);
  });

  return el;
}

async function toggleFav(clip) {
  try {
    await toggleFavorite(clip.id);
    await refresh();
  } catch (e) { console.error(e); }
}

async function selectClipAction(clip) {
  try {
    await selectClip(clip.id);
  } catch (e) { console.error(e); }
}

function showMenu(clip, event) {
  closeOpenMenu();
  _openMenuId = clip.id;

  let menu = document.getElementById(`menu-${clip.id}`);
  if (!menu) {
    menu = document.createElement("div");
    menu.id = `menu-${clip.id}`;
    menu.className = "clip-menu";
    [
      [t("action.favorite"),   () => toggleFav(clip)],
      [t("action.copy"),       () => selectClipAction(clip)],
      [t("action.delete"),     () => { deleteAction(clip); }],
    ].forEach(([label, fn]) => {
      const btn = document.createElement("button");
      btn.textContent = label;
      btn.addEventListener("click", fn);
      menu.appendChild(btn);
    });
    document.body.appendChild(menu);
  }

  menu.classList.remove("hidden");
  menu.style.left = event.clientX + "px";
  menu.style.top  = event.clientY + "px";
}

async function deleteAction(clip) {
  try {
    await deleteClip(clip.id);
    closeOpenMenu();
  } catch (e) { console.error(e); }
}

function updateSelection() {
  _parent.querySelectorAll(".clip-item").forEach((el) => {
    el.classList.toggle("selected", parseInt(el.dataset.idx) === _selectedIdx);
    if (parseInt(el.dataset.idx) === _selectedIdx) {
      el.scrollIntoView({ block: "nearest" });
    }
  });
}

// ── 工具 ──

function fmtSize(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

function formatRelativeTime(ts) {
  const diff = Date.now() - ts * 1000;
  const min = Math.floor(diff / 60000);
  if (min < 1)  return t("time.justNow");
  if (min < 60) return t("time.minutesAgo", { n: min });
  const hr = Math.floor(min / 60);
  if (hr < 24)  return t("time.hoursAgo", { n: hr });
  const days = Math.floor(hr / 24);
  if (days === 1) return t("time.yesterday");
  return t("time.daysAgo", { n: days });
}
