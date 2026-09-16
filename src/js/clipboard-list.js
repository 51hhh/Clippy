/**
 * clipboard-list.js — 剪贴板列表稳定 facade
 *
 * 状态：
 *   panelMode    : "all" | "favorites"
 *   focusedRow   : -1 | 0..N-1                  -1=无（列表空时）
 *   focusedCol   : -1 | 0 | 1 | 2               -1=行体；0..2=按钮[Copy, Favorite, Delete]
 *   expandedRow  : null | rowId                 当前展开操作组的行 id
 *
 * 数据加载与 IPC 动作留在 facade；导航状态和单行 DOM 委托给 clipboard/ 子模块。
 * 通过 telemetry 暴露关键事件用于测试与诊断。
 */

import { getClips, deleteClip, toggleFavorite, selectClip, getClipThumbnail } from "./api.ts";
import { t } from "../i18n/i18n.js";
import * as telemetry from "./telemetry.js";
import {
  collapseActions as collapseNavigationActions,
  consumePointerMove,
  createNavigationState,
  expandActions as expandNavigationActions,
  focusAction,
  focusRowBody,
  moveColumnFocus,
  moveRowFocus,
  normalizeAfterRefresh,
  releaseNavigation,
  resetForPanelChange,
} from "./clipboard/navigation-state.js";
import { createClipboardRow, syncClipboardRow } from "./clipboard/row-renderer.js";

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
let _navigation = createNavigationState();
let _dirty = false;
const PAGE_SIZE = 30;
let _hasMore = true;
let _favHasMore = true;
let _loading = false;
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
  const items = visibleItems();
  _navigation = normalizeAfterRefresh(_navigation, items.length);
  render();
  emitCounts();
  // 通知预览面板
  const focusedClip = items[_navigation.focusedRow] || null;
  _onFocusChange(focusedClip);
}

export function setQuery(q) {
  _query = q;
  refresh();
}

export function getQuery() { return _query; }

export function getFocusedClip() {
  const items = visibleItems();
  return items[_navigation.focusedRow] || null;
}

export function getLatestClip() {
  return _allClips[0] || null;
}

export async function setPanelMode(mode) {
  if (mode !== "all" && mode !== "favorites") return;
  if (_panelMode === mode) return;
  _panelMode = mode;
  _navigation = resetForPanelChange(_navigation);
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
  // 焦点跟着**条目**走，不跟着索引走（与 react/main/clipboardStore.ts::prependClip 一致，
  // 两处必须同时改，见 regression-guards.test.js）。原来这里是 `focusedRow + 1`，有两处会错：
  //   1. 面板关着时 `releaseMemory` 清空列表、`releaseNavigation` 把 focusedRow 归位。那是
  //      "没有焦点行"而不是"用户选中了第一行"，当成真焦点去 +1，重新打开面板焦点就停在
  //      **第二行**，于是截图复制完按 Pin 贴出来的是上一张图。连截两张就掉到第三行。
  //   2. 被 prepend 的条目本来就在列表里（同一张图再复制一次，`insert_clip` 按哈希去重、
  //      只把它顶到最前）时列表长度不变，+1 纯属把焦点推歪。
  // 按 id 找回原来那一行同时解决两者：真有焦点行就跟着它走（用户正看着的那行不该在眼皮下
  // 换成别的内容），没有就落在最新一条上。
  const previousFocusId = getFocusedClip()?.id ?? null;

  // 重复内容再次复制时：后端已更新 created_at 置顶，前端需移除旧位置再插入头部
  const existIdx = _allClips.findIndex((c) => c.id === clip.id);

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
  } else if (clip.is_favorite) {
    _favDirty = true;
  }

  _allClips.unshift(clip);

  // 视图里的行序变了才需要重算焦点：收藏模式下 _allClips 的变化看不见
  if (_panelMode === "all" || clip.is_favorite) {
    const items = visibleItems();
    const restored = previousFocusId === null
      ? -1
      : items.findIndex((c) => c.id === previousFocusId);
    const focusedRow = restored >= 0 ? restored : (items.length ? 0 : -1);
    _navigation = { ..._navigation, focusedRow };
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
    _updateFocusRow(_navigation.focusedRow);
  } else {
    render();
  }
  emitCounts();
}

export function removeClip(id) {
  _allClips = _allClips.filter((c) => c.id !== id);
  _favClips = _favClips.filter((c) => c.id !== id);
  _navigation = {
    ...collapseNavigationActions(_navigation),
    focusedCol: -1,
  };
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
        _navigation = { ..._navigation, focusedRow: -1 };
        render(); // 显示空状态
        _onFocusChange(null);
      } else {
        const focusedRow = Math.min(_navigation.focusedRow, items.length - 1);
        _updateFocusRow(focusedRow);
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
  const items = visibleItems();
  const transition = moveRowFocus(_navigation, delta, items.length);
  if (transition.summonSearch) {
    _onSummonSearch("keyboard");
    return;
  }
  if (transition.nextState === _navigation) return;
  _applyNavigation(transition.nextState, { syncRow: true, syncColumn: true, syncExpanded: true });
  telemetry.emit("clip-list:focus-row", { idx: _navigation.focusedRow, mode: _panelMode });
}

/** ←→/AD：在行体焦点 → 切换 panelMode；在按钮焦点 → 按钮间移动 */
export function moveCol(delta) {
  const items = visibleItems();
  const transition = moveColumnFocus(_navigation, delta, items.length, _panelMode);
  if (transition.requestedMode) {
    setPanelMode(transition.requestedMode);
    return;
  }
  _applyNavigation(transition.nextState, { syncColumn: true, syncExpanded: true });
}

/** →/D 在行体上：展开按钮组并把焦点放到第 1 个按钮 */
export function expandRowActions() {
  const items = visibleItems();
  if (items.length === 0) return;
  const clip = items[_navigation.focusedRow];
  if (!clip) return;
  const nextState = expandNavigationActions(_navigation, clip.id);
  _applyNavigation(nextState, { syncColumn: true, syncExpanded: true });
  telemetry.emit("clip-list:expand-actions", { idx: _navigation.focusedRow });
}

/** Esc / 点 ⋯ / 行外：收回按钮组 */
export function collapseActions() {
  if (_navigation.expandedRow !== null) {
    telemetry.emit("clip-list:collapse-actions", { id: _navigation.expandedRow });
  }
  _applyNavigation(collapseNavigationActions(_navigation), { syncExpanded: true });
}

/** 当前焦点是否在行体且未展开（用于 → 键决定展开还是切 tab） */
export function canExpandHere() {
  return _navigation.focusedCol === -1 && _navigation.expandedRow === null;
}

export function hasExpanded() { return _navigation.expandedRow !== null; }

export function releaseMemory() {
  _allClips = [];
  _favClips = [];
  _favDirty = true;
  _thumbCache.clear();
  _dirty = true; // 内存释放后，下次 focus 必须重新加载
  _navigation = releaseNavigation(_navigation);
  if (_parent) _parent.replaceChildren();
}

/** 标记数据已变更，下次 focus 需全量刷新 */
export function markDirty() { _dirty = true; }

/** 是否有待刷新的数据变更 */
export function isDirty() { return _dirty; }

/** 恢复渲染（无数据变更时 focus 调用，重置焦点到第一行） */
export function restoreRender() {
  if (_allClips.length > 0) {
    _navigation = resetForPanelChange(_navigation);
    render();
    _onFocusChange(visibleItems()[0] || null);
  }
}

/** Space/Enter：当前焦点（行体=复制；按钮=执行该按钮） */
export async function activateFocus(source = "keyboard") {
  const items = visibleItems();
  if (items.length === 0) return;
  const clip = items[_navigation.focusedRow];
  if (!clip) return;
  if (_navigation.focusedCol === -1) {
    return invokeAction(clip, "copy", source);
  }
  const action = ["copy", "favorite", "delete"][_navigation.focusedCol];
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

function _applyNavigation(nextState, {
  syncRow = false,
  syncColumn = false,
  syncExpanded = false,
  notifyFocus = syncRow,
} = {}) {
  const previousState = _navigation;
  _navigation = nextState;
  if (!_parent) return;

  if (syncExpanded && previousState.expandedRow !== nextState.expandedRow) {
    const oldRow = _parent.querySelector(`.clip-row[data-id="${previousState.expandedRow}"]`);
    if (oldRow) oldRow.classList.remove("expanded");
    const newRow = _parent.querySelector(`.clip-row[data-id="${nextState.expandedRow}"]`);
    if (newRow) newRow.classList.add("expanded");
  }

  if (syncRow) {
    const previousRow = _parent.querySelector(".clip-row.focused");
    if (previousRow) {
      previousRow.classList.remove("focused");
      previousRow.setAttribute("aria-selected", "false");
    }
    const focusedRow = _parent.querySelector(`.clip-row[data-idx="${nextState.focusedRow}"]`);
    if (focusedRow) {
      focusedRow.classList.add("focused");
      focusedRow.setAttribute("aria-selected", "true");
      if (typeof focusedRow.scrollIntoView === "function") {
        focusedRow.scrollIntoView({ block: "nearest" });
      }
    }
  }

  if (syncColumn) {
    const previousButton = _parent.querySelector(".clip-row-action.focused");
    if (previousButton) previousButton.classList.remove("focused");
    const row = _parent.querySelector(`.clip-row[data-idx="${nextState.focusedRow}"]`);
    const buttons = row?.querySelectorAll(".clip-row-action");
    if (buttons?.[nextState.focusedCol]) buttons[nextState.focusedCol].classList.add("focused");
  }

  if (notifyFocus) {
    _onFocusChange(visibleItems()[nextState.focusedRow] || null);
  }
}

function _updateFocusRow(focusedRow) {
  _applyNavigation(
    { ..._navigation, focusedRow },
    { syncRow: true },
  );
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
      syncClipboardRow(row, clips[idx], idx, _navigation, _panelMode);
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
  return createClipboardRow({
    clip,
    index: idx,
    navigation: _navigation,
    panelMode: _panelMode,
    thumbnailCache: _thumbCache,
    maxThumbnailCache: MAX_THUMB_CACHE,
    // 缩略图而不是原图：行里那格 48×48，取原图等于把几 MB 的 PNG 送进 webview
    // 再全尺寸解码（后端 get_clip_thumbnail 已经缩到 128 px 并缓存）。
    loadThumbnail: getClipThumbnail,
    onAction: (targetClip, action, rowIndex, actionIndex) => {
      const nextState = actionIndex === -1
        ? focusRowBody(_navigation, rowIndex)
        : focusAction(_navigation, rowIndex, actionIndex);
      _applyNavigation(nextState, { syncRow: true, syncColumn: true });
      invokeAction(targetClip, action, "mouse");
    },
    onToggleActions: (targetClip, rowIndex) => {
      const wasExpanded = _navigation.expandedRow === targetClip.id;
      _applyNavigation(
        focusRowBody(_navigation, rowIndex),
        { syncRow: true, syncColumn: true },
      );
      if (wasExpanded) collapseActions();
      else expandRowActions();
    },
    onPointerFocus: (_targetClip, rowIndex) => {
      const pointerTransition = consumePointerMove(_navigation);
      _navigation = pointerTransition.nextState;
      if (pointerTransition.ignore || rowIndex === _navigation.focusedRow) return;
      _applyNavigation(
        focusRowBody(_navigation, rowIndex),
        { syncRow: true, syncColumn: true },
      );
    },
  });
}

// ── 测试用辅助导出（不进生产文档但内部状态可观察） ──
export const __test__ = {
  state: () => ({
    panelMode: _panelMode,
    focusedRow: _navigation.focusedRow,
    focusedCol: _navigation.focusedCol,
    expandedRow: _navigation.expandedRow,
  }),
  reset() {
    _query = ""; _panelMode = "all"; _allClips = []; _favClips = []; _favDirty = true;
    _hasMore = true; _favHasMore = true; _thumbCache.clear();
    _navigation = createNavigationState();
  },
};
