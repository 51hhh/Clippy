/**
 * clipboard-list.js — 剪贴板列表 + 行内 ⋯ 操作组 状态机
 *
 * 状态：
 *   panelMode    : "all" | "favorites"
 *   focusedRow   : -1 | 0..N-1                  -1=无（列表空时）
 *   focusedCol   : -1 | 0 | 1 | 2               -1=行体；0..2=按钮[Copy, Favorite, Delete]
 *   expandedRow  : null | rowId                 当前展开操作组的行 id
 *   deletePending: { rowId, expiresAt } | null  删除二次确认
 *
 * 通过 telemetry 暴露关键事件用于测试与诊断。
 */

import { getClips, deleteClip, toggleFavorite, selectClip } from "./api.js";
import { t } from "../i18n/i18n.js";
import * as telemetry from "./telemetry.js";

let DELETE_CONFIRM_MS = 1200; // 默认值，从 config 覆盖

let _parent = null;
let _emptyEl = null;
let _onCountsChange = () => {};
let _onSummonSearch = () => {};

let _query = "";
let _panelMode = "all";
let _allClips = [];
let _focusedRow = -1;
let _focusedCol = -1;
let _expandedRow = null;
let _deletePending = null;
let _deleteTimer = null;

export function init({ listEl, emptyEl, onCountsChange, onSummonSearch }) {
  _parent = listEl;
  _emptyEl = emptyEl;
  _onCountsChange = onCountsChange || (() => {});
  _onSummonSearch = onSummonSearch || (() => {});
}

export function setDeleteConfirmMs(ms) {
  DELETE_CONFIRM_MS = ms;
}

export async function refresh() {
  try {
    _allClips = await getClips(_query || null, false, 0, 200);
    telemetry.emit("clip-list:refresh", {
      query: _query, mode: _panelMode, count: _allClips.length,
    });
  } catch (e) {
    console.error("Clipboard query failed:", e);
    _allClips = [];
    telemetry.emit("clip-list:refresh-error", { message: String(e) });
  }
  // 焦点收敛：刷新后保持在合法范围；列表空则 -1
  const items = visibleItems();
  if (items.length === 0) _focusedRow = -1;
  else _focusedRow = Math.max(0, Math.min(_focusedRow, items.length - 1));
  if (_focusedRow === -1) _focusedRow = 0;
  _focusedCol = -1;
  _expandedRow = null;
  cancelDeletePending();
  render();
  emitCounts();
}

export function setQuery(q) {
  _query = q;
  refresh();
}

export function getQuery() { return _query; }

export function setPanelMode(mode) {
  if (mode !== "all" && mode !== "favorites") return;
  if (_panelMode === mode) return;
  _panelMode = mode;
  _focusedRow = 0;
  _focusedCol = -1;
  _expandedRow = null;
  cancelDeletePending();
  render();
  emitCounts();
  telemetry.emit("clip-list:set-mode", { mode });
}

export function getPanelMode() { return _panelMode; }

export function prependClip(clip) {
  _allClips.unshift(clip);
  if (_focusedRow >= 0) _focusedRow += 1;
  render();
  emitCounts();
}

export function removeClip(id) {
  _allClips = _allClips.filter((c) => c.id !== id);
  render();
  emitCounts();
}

// ── 焦点 / 操作 ─────────────────────────────────────────────

/** ↑↓/WS：竖直选行；当前在按钮列时仍按行体处理（先收回） */
export function moveRow(delta) {
  // 行体焦点在第 0 行，再 ↑ → 召唤搜索
  const items = visibleItems();
  if (items.length === 0) return;
  if (delta < 0 && _focusedRow === 0 && _focusedCol === -1) {
    _onSummonSearch("keyboard");
    return;
  }
  // 移动行：自动退出按钮区
  _focusedCol = -1;
  collapseActions();
  _focusedRow = clamp(_focusedRow + delta, 0, items.length - 1);
  render();
  telemetry.emit("clip-list:focus-row", { idx: _focusedRow, mode: _panelMode });
}

/** ←→/AD：在行体焦点 → 切换 panelMode；在按钮焦点 → 按钮间移动 */
export function moveCol(delta) {
  const items = visibleItems();
  if (items.length === 0) return;
  if (_focusedCol === -1) {
    // 行体：横向 = 切 tab
    if (delta < 0 && _panelMode === "all") {
      setPanelMode("favorites");
    } else if (delta > 0 && _panelMode === "favorites") {
      setPanelMode("all");
    }
    return;
  }
  // 按钮区：按钮间移动
  const next = _focusedCol + delta;
  if (next < 0) {
    // 最左再 ← → 收回
    collapseActions();
    _focusedCol = -1;
    render();
    return;
  }
  if (next > 2) return; // 最右无效
  _focusedCol = next;
  render();
}

/** →/D 在行体上：展开按钮组并把焦点放到第 1 个按钮 */
export function expandRowActions() {
  const items = visibleItems();
  if (items.length === 0) return;
  const clip = items[_focusedRow];
  if (!clip) return;
  _expandedRow = clip.id;
  _focusedCol = 0;
  render();
  telemetry.emit("clip-list:expand-actions", { idx: _focusedRow });
}

/** Esc / 点 ⋯ / 行外：收回按钮组 */
export function collapseActions() {
  if (_expandedRow !== null) {
    telemetry.emit("clip-list:collapse-actions", { id: _expandedRow });
  }
  _expandedRow = null;
  cancelDeletePending();
  render();
}

/** 当前焦点是否在行体且未展开（用于 → 键决定展开还是切 tab） */
export function canExpandHere() {
  return _focusedCol === -1 && _expandedRow === null;
}

export function hasExpanded() { return _expandedRow !== null; }

export function releaseMemory() {
  _allClips = [];
  if (_parent) _parent.replaceChildren();
}

/** Space/Enter：当前焦点（行体=复制；按钮=执行该按钮） */
export async function activateFocus(source = "keyboard") {
  const items = visibleItems();
  if (items.length === 0) return;
  const clip = items[_focusedRow];
  if (!clip) return;
  if (_focusedCol === -1) {
    return invokeAction(clip, "copy", source);
  }
  const action = ["copy", "favorite", "delete"][_focusedCol];
  return invokeAction(clip, action, source);
}

async function invokeAction(clip, action, source) {
  try {
    if (action === "copy") {
      await selectClip(clip.id);
    } else if (action === "favorite") {
      await toggleFavorite(clip.id);
      await refresh();
    } else if (action === "delete") {
      // 二次确认
      if (!_deletePending || _deletePending.rowId !== clip.id || Date.now() > _deletePending.expiresAt) {
        _deletePending = { rowId: clip.id, expiresAt: Date.now() + DELETE_CONFIRM_MS };
        clearTimeout(_deleteTimer);
        _deleteTimer = setTimeout(() => {
          _deletePending = null;
          render();
        }, DELETE_CONFIRM_MS);
        render();
        return;
      }
      cancelDeletePending();
      await deleteClip(clip.id);
    }
    telemetry.emit("clip-list:invoke-action", { action, source });
  } catch (err) {
    console.error("invokeAction 失败:", err);
    telemetry.emit("clip-list:invoke-action-error", { action, message: String(err) });
  }
}

function cancelDeletePending() {
  if (_deleteTimer) clearTimeout(_deleteTimer);
  _deleteTimer = null;
  _deletePending = null;
}

// ── 内部工具 ─────────────────────────────────────────────────

function visibleItems() {
  if (_panelMode === "favorites") return _allClips.filter((c) => c.is_favorite);
  return _allClips;
}

function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }

function emitCounts() {
  const all = _allClips.length;
  const favorites = _allClips.filter((c) => c.is_favorite).length;
  _onCountsChange({ all, favorites });
}

// ── 渲染 ─────────────────────────────────────────────────────

function render() {
  if (!_parent) return;
  const clips = visibleItems();

  if (clips.length === 0) {
    _parent.replaceChildren();
    if (_emptyEl) {
      _emptyEl.hidden = false;
      const txt = _emptyEl.querySelector("#empty-state-text");
      if (txt) {
        txt.textContent = t(_panelMode === "favorites" ? "empty.favorites" : "empty.text");
      }
    }
    telemetry.emit("clip-list:render", { count: 0, empty: true, mode: _panelMode });
    return;
  }
  if (_emptyEl) _emptyEl.hidden = true;

  _parent.replaceChildren();
  clips.forEach((clip, idx) => {
    _parent.appendChild(buildRow(clip, idx));
  });
  // scroll focus into view
  const focused = _parent.querySelector(".clip-row.focused");
  if (focused && typeof focused.scrollIntoView === "function") {
    focused.scrollIntoView({ block: "nearest" });
  }
  telemetry.emit("clip-list:render", { count: clips.length, empty: false, mode: _panelMode });
}

function buildRow(clip, idx) {
  const row = document.createElement("article");
  row.className = "clip-row";
  if (idx === _focusedRow) row.classList.add("focused");
  if (clip.is_favorite) row.classList.add("favorite");
  if (_expandedRow === clip.id) row.classList.add("expanded");
  row.dataset.idx = idx;
  row.dataset.id = clip.id;
  row.setAttribute("role", "option");
  row.setAttribute("aria-selected", String(idx === _focusedRow));

  // stripe + favorite 标记由 CSS 处理（::before）

  const main = document.createElement("div");
  main.className = "clip-row-main";

  const preview = document.createElement("div");
  preview.className = "clip-row-preview";
  preview.textContent = (clip.text_content || "").slice(0, 200);

  const meta = document.createElement("div");
  meta.className = "clip-row-meta";
  meta.textContent = `${formatRelativeTime(clip.created_at)} · ${formatType(clip.content_type)} · ${fmtSize(clip.byte_size)}`;

  main.append(preview, meta);
  row.appendChild(main);

  // 操作组（默认 collapsed）
  const actions = document.createElement("div");
  actions.className = "clip-row-actions";
  const buttons = [
    { key: "copy",     label: t("action.copy"),     icon: "⎘" },
    { key: "favorite", label: clip.is_favorite ? t("action.unfavorite") : t("action.favorite"), icon: clip.is_favorite ? "★" : "☆" },
    { key: "delete",   label: t("action.delete"),   icon: "✕" },
  ];
  buttons.forEach((b, btnIdx) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "clip-row-action";
    if (b.key === "favorite" && clip.is_favorite) btn.classList.add("is-favorite");
    if (b.key === "delete" && _deletePending?.rowId === clip.id) btn.classList.add("danger-confirm");
    if (idx === _focusedRow && _focusedCol === btnIdx) btn.classList.add("focused");
    btn.dataset.action = b.key;
    btn.setAttribute("aria-label", b.label);
    btn.title = b.label;

    const icon = document.createElement("span");
    icon.className = "clip-row-action-icon";
    icon.textContent = b.icon;
    btn.appendChild(icon);

    if (b.key === "delete" && _deletePending?.rowId === clip.id) {
      const txt = document.createElement("span");
      txt.className = "clip-row-action-confirm";
      txt.textContent = t("action.confirm");
      btn.appendChild(txt);
    }

    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      _focusedRow = idx;
      _focusedCol = btnIdx;
      invokeAction(clip, b.key, "mouse");
    });
    actions.appendChild(btn);
  });

  // ⋯ 触发器
  const trigger = document.createElement("button");
  trigger.type = "button";
  trigger.className = "clip-row-trigger";
  trigger.setAttribute("aria-label", t("action.more"));
  trigger.title = t("action.more");
  trigger.textContent = "⋯";
  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    _focusedRow = idx;
    if (_expandedRow === clip.id) {
      collapseActions();
      _focusedCol = -1;
      render();
    } else {
      expandRowActions();
    }
  });

  row.append(actions, trigger);

  // 行体点击 = copy（触发器与按钮已 stopPropagation）
  row.addEventListener("click", () => {
    _focusedRow = idx;
    _focusedCol = -1;
    invokeAction(clip, "copy", "mouse");
  });
  row.addEventListener("mouseenter", () => {
    if (idx !== _focusedRow) {
      _focusedRow = idx;
      _focusedCol = -1;
      render();
    }
  });

  return row;
}

// ── 工具 ─────────────────────────────────────────────────────

function fmtSize(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

function formatType(type) {
  if (!type) return "Text";
  if (type === "text") return "Text";
  if (type === "html") return "HTML";
  if (type === "image") return "Image";
  return type;
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

// ── 测试用辅助导出（不进生产文档但内部状态可观察） ──
export const __test__ = {
  state: () => ({
    panelMode: _panelMode,
    focusedRow: _focusedRow,
    focusedCol: _focusedCol,
    expandedRow: _expandedRow,
    deletePending: _deletePending ? { ..._deletePending } : null,
  }),
  reset() {
    _query = ""; _panelMode = "all"; _allClips = [];
    _focusedRow = -1; _focusedCol = -1; _expandedRow = null;
    cancelDeletePending();
  },
};
