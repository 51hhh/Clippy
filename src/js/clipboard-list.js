/**
 * clipboard-list.js — 剪贴板列表 + 行内 ⋯ 操作组 状态机
 *
 * 状态：
 *   panelMode    : "all" | "favorites"
 *   focusedRow   : -1 | 0..N-1                  -1=无（列表空时）
 *   focusedCol   : -1 | 0 | 1 | 2               -1=行体；0..2=按钮[Copy, Favorite, Delete]
 *   expandedRow  : null | rowId                 当前展开操作组的行 id
 *
 * 通过 telemetry 暴露关键事件用于测试与诊断。
 */

import { getClips, deleteClip, toggleFavorite, selectClip, getClipImage } from "./api.js";
import { t } from "../i18n/i18n.js";
import * as telemetry from "./telemetry.js";
import * as icons from "./icons.js";

let _parent = null;
let _emptyEl = null;
let _onCountsChange = () => {};
let _onSummonSearch = () => {};
let _onModeChange = () => {};
let _onFocusChange = () => {};

let _query = "";
let _panelMode = "all";
let _allClips = [];
let _favClips = [];
let _favDirty = true; // 收藏列表需要重新加载
let _focusedRow = -1;
let _focusedCol = -1;
let _expandedRow = null;
let _dirty = false;
const PAGE_SIZE = 30;
let _hasMore = true;
let _favHasMore = true;
let _loading = false;
let _keyboardNav = false; // 键盘导航中，鼠标悬浮不抢焦点
const _thumbCache = new Map();
const MAX_THUMB_CACHE = 50;
export function init({ listEl, emptyEl, onCountsChange, onSummonSearch, onModeChange, onFocusChange }) {
  _parent = listEl;
  _emptyEl = emptyEl;
  _onCountsChange = onCountsChange || (() => {});
  _onSummonSearch = onSummonSearch || (() => {});
  _onModeChange = onModeChange || (() => {});
  _onFocusChange = onFocusChange || (() => {});
  // 滚动到底部时追加加载
  if (_parent) {
    _parent.addEventListener("scroll", _onScroll);
  }
}

export async function refresh() {
  _dirty = false;
  _hasMore = true;
  _favDirty = true;
  try {
    _allClips = await getClips(_query || null, false, 0, PAGE_SIZE);
    _hasMore = _allClips.length >= PAGE_SIZE;
    telemetry.emit("clip-list:refresh", {
      query: _query, mode: _panelMode, count: _allClips.length,
    });
  } catch (e) {
    console.error("Clipboard query failed:", e);
    _allClips = [];
    telemetry.emit("clip-list:refresh-error", { message: String(e) });
  }
  // 如果当前在收藏模式，同步加载收藏列表
  if (_panelMode === "favorites") {
    await _loadFavorites();
  }
  // 焦点收敛：刷新后保持在合法范围；列表空则 -1
  const items = visibleItems();
  if (items.length === 0) _focusedRow = -1;
  else _focusedRow = Math.max(0, Math.min(_focusedRow, items.length - 1));
  _focusedCol = -1;
  _expandedRow = null;
  render();
  emitCounts();
  // 通知预览面板
  const focusedClip = items[_focusedRow] || null;
  _onFocusChange(focusedClip);
}

export function setQuery(q) {
  _query = q;
  refresh();
}

export function getQuery() { return _query; }

export function getFocusedClip() {
  const items = visibleItems();
  return items[_focusedRow] || null;
}

export function getLatestClip() {
  return _allClips[0] || null;
}

export async function setPanelMode(mode) {
  if (mode !== "all" && mode !== "favorites") return;
  if (_panelMode === mode) return;
  _panelMode = mode;
  _focusedRow = 0;
  _focusedCol = -1;
  _expandedRow = null;
  // 切换淡入动画
  if (_parent) {
    _parent.classList.remove("switch-left", "switch-right");
    void _parent.offsetWidth;
    _parent.classList.add("switch-left");
  }
  // 收藏列表独立加载
  if (mode === "favorites" && _favDirty) {
    await _loadFavorites();
  }
  render();
  emitCounts();
  _onModeChange(mode);
  telemetry.emit("clip-list:set-mode", { mode });
}

export function getPanelMode() { return _panelMode; }

export function prependClip(clip) {
  // 重复内容再次复制时：后端已更新 created_at 置顶，前端需移除旧位置再插入头部
  const existIdx = _allClips.findIndex((c) => c.id === clip.id);
  const focusOnMoved = existIdx !== -1 && _focusedRow === existIdx;

  if (existIdx !== -1) {
    _allClips.splice(existIdx, 1);
    // 同步更新收藏列表中的旧对象（保留收藏状态，刷新 created_at）
    const favIdx = _favClips.findIndex((c) => c.id === clip.id);
    if (favIdx !== -1) _favClips[favIdx] = clip;
    if (clip.is_favorite) _favDirty = true;
    // 移除旧 DOM 节点并重新编号
    if (!_query && _panelMode === "all" && _parent) {
      const oldEl = _parent.querySelector(`.clip-row[data-id="${clip.id}"]`);
      if (oldEl) oldEl.remove();
      let i = 0;
      for (const row of _parent.children) {
        row.dataset.idx = i++;
      }
    }
    // 修正焦点：splice 后 existIdx 之后的项前移一位
    if (_focusedRow > existIdx) _focusedRow -= 1;
  } else if (clip.is_favorite) {
    _favDirty = true;
  }

  _allClips.unshift(clip);
  // 焦点原本在被移动条目上 → 它已到 index 0，不走普通 +1 路径
  if (focusOnMoved) {
    _focusedRow = 0;
  } else if (_focusedRow >= 0 && _panelMode === "all") {
    _focusedRow += 1;
  }

  // 差量更新：若当前视图匹配（非搜索模式），直接 prepend DOM 节点
  if (!_query && _panelMode === "all" && _parent && _parent.children.length > 0) {
    // 更新现有行的 idx
    for (const row of _parent.children) {
      const oldIdx = parseInt(row.dataset.idx, 10);
      row.dataset.idx = oldIdx + 1;
    }
    const newRow = buildRow(clip, 0);
    _parent.prepend(newRow);
    if (_emptyEl) _emptyEl.hidden = true;
    // 更新焦点
    _updateFocusRow(_focusedRow);
  } else {
    render();
  }
  emitCounts();
}

export function removeClip(id) {
  _allClips = _allClips.filter((c) => c.id !== id);
  _favClips = _favClips.filter((c) => c.id !== id);
  _expandedRow = null;
  _focusedCol = -1;
  // 差量更新：直接移除 DOM 节点
  if (!_query && _parent) {
    const el = _parent.querySelector(`.clip-row[data-id="${id}"]`);
    if (el) {
      el.remove();
      // 重新编号 idx
      let i = 0;
      for (const row of _parent.children) {
        row.dataset.idx = i++;
      }
      // 修正焦点
      const items = visibleItems();
      if (items.length === 0) {
        _focusedRow = -1;
        render(); // 显示空状态
        _onFocusChange(null);
      } else {
        _focusedRow = Math.min(_focusedRow, items.length - 1);
        _updateFocusRow(_focusedRow);
      }
    }
  } else {
    render();
  }
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
  // 标记键盘导航中，鼠标不抢焦点
  _keyboardNav = true;
  // 移动行：自动退出按钮区
  _focusedCol = -1;
  if (_expandedRow !== null) {
    _updateExpanded(null);
  }
  const newRow = clamp(_focusedRow + delta, 0, items.length - 1);
  _updateFocusRow(newRow);
  telemetry.emit("clip-list:focus-row", { idx: _focusedRow, mode: _panelMode });
}

/** ←→/AD：在行体焦点 → 切换 panelMode；在按钮焦点 → 按钮间移动 */
export function moveCol(delta) {
  const items = visibleItems();
  if (_focusedCol === -1) {
    if (delta < 0 && _panelMode === "all") {
      setPanelMode("favorites");
    } else if (delta > 0 && _panelMode === "favorites") {
      setPanelMode("all");
    }
    return;
  }
  if (items.length === 0) return;
  // 按钮区：按钮间移动
  // 收藏模式按钮在左侧，方向反转
  const effectiveDelta = _panelMode === "favorites" ? -delta : delta;
  const next = _focusedCol + effectiveDelta;
  if (next < 0) {
    _updateExpanded(null);
    _focusedCol = -1;
    return;
  }
  if (next > 2) return;
  _updateFocusCol(next);
}

/** →/D 在行体上：展开按钮组并把焦点放到第 1 个按钮 */
export function expandRowActions() {
  const items = visibleItems();
  if (items.length === 0) return;
  const clip = items[_focusedRow];
  if (!clip) return;
  _updateExpanded(clip.id);
  _updateFocusCol(0);
  telemetry.emit("clip-list:expand-actions", { idx: _focusedRow });
}

/** Esc / 点 ⋯ / 行外：收回按钮组 */
export function collapseActions() {
  if (_expandedRow !== null) {
    telemetry.emit("clip-list:collapse-actions", { id: _expandedRow });
  }
  _updateExpanded(null);
}

/** 当前焦点是否在行体且未展开（用于 → 键决定展开还是切 tab） */
export function canExpandHere() {
  return _focusedCol === -1 && _expandedRow === null;
}

export function hasExpanded() { return _expandedRow !== null; }

export function releaseMemory() {
  _allClips = [];
  _favClips = [];
  _favDirty = true;
  _thumbCache.clear();
  _dirty = true; // 内存释放后，下次 focus 必须重新加载
  _focusedRow = 0; // 下次打开时从第一项（最新）开始
  _focusedCol = -1;
  _expandedRow = null;
  if (_parent) _parent.replaceChildren();
}

/** 标记数据已变更，下次 focus 需全量刷新 */
export function markDirty() { _dirty = true; }

/** 是否有待刷新的数据变更 */
export function isDirty() { return _dirty; }

/** 恢复渲染（无数据变更时 focus 调用，重置焦点到第一行） */
export function restoreRender() {
  if (_allClips.length > 0) {
    _focusedRow = 0;
    _focusedCol = -1;
    _expandedRow = null;
    render();
    _onFocusChange(visibleItems()[0] || null);
  }
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

/** 数字键直选：按索引选中条目并粘贴（0-based） */
export async function selectByIndex(index) {
  const items = visibleItems();
  if (index < 0 || index >= items.length) return false;
  const clip = items[index];
  if (!clip) return false;
  await invokeAction(clip, "copy", "number-key");
  return true;
}

async function invokeAction(clip, action, source) {
  try {
    if (action === "copy") {
      await selectClip(clip.id);
    } else if (action === "favorite") {
      await toggleFavorite(clip.id);
      await refresh();
    } else if (action === "delete") {
      await deleteClip(clip.id);
      removeClip(clip.id);
    }
    telemetry.emit("clip-list:invoke-action", { action, source });
  } catch (err) {
    console.error("invokeAction 失败:", err);
    telemetry.emit("clip-list:invoke-action-error", { action, message: String(err) });
  }
}


// ── 内部工具 ─────────────────────────────────────────────────

function visibleItems() {
  if (_panelMode === "favorites") return _favClips;
  return _allClips;
}

function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }

/** 独立加载收藏列表 */
async function _loadFavorites() {
  try {
    _favClips = await getClips(_query || null, true, 0, PAGE_SIZE);
    _favHasMore = _favClips.length >= PAGE_SIZE;
    _favDirty = false;
  } catch (e) {
    console.error("Favorites query failed:", e);
    _favClips = [];
  }
}

/** 滚动到底部时追加加载 */
async function _onScroll() {
  if (!_parent || _loading) return;
  const isFav = _panelMode === "favorites";
  const hasMore = isFav ? _favHasMore : _hasMore;
  if (!hasMore) return;
  const { scrollTop, scrollHeight, clientHeight } = _parent;
  if (scrollTop + clientHeight < scrollHeight - 50) return;
  _loading = true;
  try {
    const arr = isFav ? _favClips : _allClips;
    const more = await getClips(_query || null, isFav, arr.length, PAGE_SIZE);
    if (more.length > 0) {
      const startIdx = arr.length;
      arr.push(...more);
      // 差量追加 DOM
      more.forEach((clip, i) => {
        _parent.appendChild(buildRow(clip, startIdx + i));
      });
      emitCounts();
    }
    if (isFav) _favHasMore = more.length >= PAGE_SIZE;
    else _hasMore = more.length >= PAGE_SIZE;
  } catch (e) {
    console.error("追加加载失败:", e);
  }
  _loading = false;
}

function emitCounts() {
  const all = _allClips.length;
  // 收藏计数：如果已加载则用独立列表，否则从 allClips 估算
  const favorites = _favDirty
    ? _allClips.filter((c) => c.is_favorite).length
    : _favClips.length;
  _onCountsChange({ all, favorites });
}

// ── 差量 DOM 更新辅助 ───────────────────────────────────────

/** 仅切换焦点行的 CSS class，不重建 DOM */
function _updateFocusRow(newRow) {
  if (!_parent) return;
  const prev = _parent.querySelector(".clip-row.focused");
  if (prev) {
    prev.classList.remove("focused");
    prev.setAttribute("aria-selected", "false");
  }
  _focusedRow = newRow;
  const next = _parent.querySelector(`.clip-row[data-idx="${newRow}"]`);
  if (next) {
    next.classList.add("focused");
    next.setAttribute("aria-selected", "true");
    if (typeof next.scrollIntoView === "function") {
      next.scrollIntoView({ block: "nearest" });
    }
  }
  // 通知预览面板焦点变化
  const items = visibleItems();
  _onFocusChange(items[newRow] || null);
}

/** 仅切换按钮焦点的 CSS class */
function _updateFocusCol(newCol) {
  if (!_parent) return;
  // 移除旧按钮焦点
  const prevBtn = _parent.querySelector(".clip-row-action.focused");
  if (prevBtn) prevBtn.classList.remove("focused");
  _focusedCol = newCol;
  // 设置新按钮焦点
  const row = _parent.querySelector(`.clip-row[data-idx="${_focusedRow}"]`);
  if (row) {
    const btns = row.querySelectorAll(".clip-row-action");
    if (btns[newCol]) btns[newCol].classList.add("focused");
  }
}

/** 切换行的展开/收起状态 */
function _updateExpanded(newExpandedId) {
  if (!_parent) return;
  // 收起旧的
  if (_expandedRow !== null) {
    const old = _parent.querySelector(`.clip-row[data-id="${_expandedRow}"]`);
    if (old) old.classList.remove("expanded");
  }
  _expandedRow = newExpandedId;
  // 展开新的
  if (newExpandedId !== null) {
    const el = _parent.querySelector(`.clip-row[data-id="${newExpandedId}"]`);
    if (el) el.classList.add("expanded");
  }
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

  // 差量更新：如果现有 DOM 行的 id 序列匹配，只更新焦点/状态 class
  const existingRows = _parent.querySelectorAll(".clip-row");
  const idsMatch = existingRows.length === clips.length &&
    clips.every((clip, i) => existingRows[i].dataset.id === String(clip.id));

  if (idsMatch) {
    existingRows.forEach((row, idx) => {
      const clip = clips[idx];
      row.classList.toggle("focused", idx === _focusedRow);
      row.classList.toggle("expanded", _expandedRow === clip.id);
      row.classList.toggle("favorite", !!clip.is_favorite);
      const favBtn = row.querySelector('[data-action="favorite"]');
      if (favBtn) {
        favBtn.classList.toggle("is-favorite", !!clip.is_favorite);
        const iconEl = favBtn.querySelector(".clip-row-action-icon");
        if (iconEl) iconEl.innerHTML = clip.is_favorite ? icons.starFill : icons.star;
        favBtn.setAttribute("aria-label", clip.is_favorite ? t("action.unfavorite") : t("action.favorite"));
        favBtn.title = clip.is_favorite ? t("action.unfavorite") : t("action.favorite");
      }
    });
  } else {
    _parent.replaceChildren();
    clips.forEach((clip, idx) => {
      _parent.appendChild(buildRow(clip, idx));
    });
  }
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
  if (_panelMode === "favorites") row.classList.add("favorites-mode");
  row.dataset.idx = idx;
  row.dataset.id = clip.id;
  row.setAttribute("role", "option");
  row.setAttribute("aria-selected", String(idx === _focusedRow));

  // stripe + favorite 标记由 CSS 处理（::before）

  const main = document.createElement("div");
  main.className = "clip-row-main";

  const preview = document.createElement("div");
  preview.className = "clip-row-preview";

  if (clip.content_type === "image") {
    preview.classList.add("clip-row-preview--image");
    const thumb = document.createElement("div");
    thumb.className = "clip-row-thumb";
    thumb.textContent = "🖼";
    // 异步加载缩略图（带缓存）
    const cached = _thumbCache.get(clip.id);
    if (cached) {
      const img = document.createElement("img");
      img.src = `data:image/png;base64,${cached}`;
      img.alt = "image";
      img.className = "clip-row-thumb-img";
      img.draggable = false;
      thumb.textContent = "";
      thumb.appendChild(img);
    } else {
      getClipImage(clip.id).then(base64 => {
        if (base64) {
          if (_thumbCache.size >= MAX_THUMB_CACHE) {
            const oldest = _thumbCache.keys().next().value;
            _thumbCache.delete(oldest);
          }
          _thumbCache.set(clip.id, base64);
          const img = document.createElement("img");
          img.src = `data:image/png;base64,${base64}`;
          img.alt = "image";
          img.className = "clip-row-thumb-img";
          img.draggable = false;
          thumb.textContent = "";
          thumb.appendChild(img);
        }
      }).catch(() => {});
    }
    preview.appendChild(thumb);
  } else if (clip.content_type === "html") {
    preview.textContent = (clip.text_content || t("preview.richText")).slice(0, 200);
  } else {
    preview.textContent = (clip.text_content || "").slice(0, 200);
  }

  const meta = document.createElement("div");
  meta.className = "clip-row-meta";
  const metaParts = [`${formatRelativeTime(clip.created_at)} · ${formatType(clip.content_type)} · ${fmtSize(clip.byte_size)}`];
  if (clip.is_sensitive) {
    row.classList.add("sensitive");
    metaParts.unshift("🔒");
  }
  meta.textContent = metaParts.join(" ");

  main.append(preview, meta);

  // 操作组（默认 collapsed）
  const actions = document.createElement("div");
  actions.className = "clip-row-actions";
  const buttons = [
    { key: "copy",     label: t("action.copy"),     icon: icons.copy },
    { key: "favorite", label: clip.is_favorite ? t("action.unfavorite") : t("action.favorite"), icon: clip.is_favorite ? icons.starFill : icons.star },
    { key: "delete",   label: t("action.delete"),   icon: icons.trash },
  ];
  buttons.forEach((b, btnIdx) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "clip-row-action";
    if (b.key === "favorite" && clip.is_favorite) btn.classList.add("is-favorite");
    if (idx === _focusedRow && _focusedCol === btnIdx) btn.classList.add("focused");
    btn.dataset.action = b.key;
    btn.setAttribute("aria-label", b.label);
    btn.title = b.label;

    const icon = document.createElement("span");
    icon.className = "clip-row-action-icon";
    icon.innerHTML = b.icon;
    btn.appendChild(icon);

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
  trigger.innerHTML = icons.more;
  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    _focusedRow = idx;
    if (_expandedRow === clip.id) {
      collapseActions();
      _focusedCol = -1;
    } else {
      expandRowActions();
    }
  });

  // 收藏模式：trigger + actions 在左侧
  if (_panelMode === "favorites") {
    row.append(trigger, actions, main);
  } else {
    row.append(main, actions, trigger);
  }

  // 行体点击 = copy（触发器与按钮已 stopPropagation）
  row.addEventListener("click", () => {
    _focusedRow = idx;
    _focusedCol = -1;
    invokeAction(clip, "copy", "mouse");
  });
  row.addEventListener("mousemove", () => {
    // 键盘导航期间忽略鼠标悬浮（需要实际移动鼠标才激活）
    if (_keyboardNav) {
      _keyboardNav = false;
      return;
    }
    if (idx !== _focusedRow) {
      // 只切换 CSS class，不全量重建 DOM
      const prev = _parent?.querySelector(".clip-row.focused");
      if (prev) prev.classList.remove("focused");
      row.classList.add("focused");
      _focusedRow = idx;
      _focusedCol = -1;
      _onFocusChange(clip);
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
  }),
  reset() {
    _query = ""; _panelMode = "all"; _allClips = []; _favClips = []; _favDirty = true;
    _hasMore = true; _favHasMore = true; _thumbCache.clear();
    _focusedRow = -1; _focusedCol = -1; _expandedRow = null;
  },
};
