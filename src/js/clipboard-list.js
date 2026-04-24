/**
 * clipboard-list.js — 剪贴板列表模块
 * 负责条目渲染、分页加载、键盘导航、操作菜单。
 */

import { getClips, deleteClip, toggleFavorite, selectClip } from "./api.js";

// ── 常量 ───────────────────────────────────────────────────────────────────
const PAGE_SIZE = 20;

// ── 模块状态 ───────────────────────────────────────────────────────────────
let _container   = null;
let _emptyState  = null;

let clips         = [];
let offset        = 0;
let currentQuery  = null;
let favoritesOnly = false;
let selectedIndex = -1;
let loading       = false;
let hasMore       = true;

// 当前打开的操作菜单 DOM 节点（用于关闭处理）
let _openMenu = null;

// ── 初始化 ─────────────────────────────────────────────────────────────────

/**
 * 初始化模块，绑定容器并启动无限滚动监听。
 * @param {HTMLElement} container
 * @param {HTMLElement} emptyState
 */
export function init(container, emptyState) {
  _container  = container;
  _emptyState = emptyState;

  _container.addEventListener("scroll", _onScroll);
}

// ── 公开 API ───────────────────────────────────────────────────────────────

/** 重置并重新加载第一页。 */
export async function refresh() {
  clips         = [];
  offset        = 0;
  selectedIndex = -1;
  hasMore       = true;
  closeOpenMenu();
  _clearList();
  await _loadPage();
}

/** 更新搜索词并刷新列表。 */
export function setQuery(query) {
  currentQuery = query || null;
  refresh();
}

/** 切换"仅收藏"过滤并刷新列表。 */
export function setFavoritesOnly(fav) {
  favoritesOnly = fav;
  refresh();
}

/**
 * 将新条目插入列表顶部（去重）。
 * @param {ClipItem} clip
 */
export function prependClip(clip) {
  // 去重
  const existing = clips.findIndex((c) => c.id === clip.id);
  if (existing !== -1) {
    clips.splice(existing, 1);
    const oldEl = _container.querySelector(`[data-id="${clip.id}"]`);
    if (oldEl) oldEl.remove();
  }

  clips.unshift(clip);
  offset = Math.max(0, offset + 1); // 保持偏移量同步

  const el = createClipElement(clip);
  const firstItem = _container.querySelector(".clip-item");
  if (firstItem) {
    _container.insertBefore(el, firstItem);
  } else {
    // 移除空状态后插入
    _container.appendChild(el);
  }
  _updateEmptyState();
}

/**
 * 从列表中移除指定条目。
 * @param {number} clipId
 */
export function removeClip(clipId) {
  const idx = clips.findIndex((c) => c.id === clipId);
  if (idx === -1) return;

  clips.splice(idx, 1);
  offset = Math.max(0, offset - 1);

  const el = _container.querySelector(`[data-id="${clipId}"]`);
  if (el) el.remove();

  // 修正选中索引
  if (selectedIndex >= clips.length) {
    selectedIndex = clips.length - 1;
  }
  _applySelection();
  _updateEmptyState();
}

/**
 * 键盘导航：上 (-1) / 下 (+1)。
 * @param {number} direction
 */
export function moveSelection(direction) {
  if (clips.length === 0) return;

  const next = selectedIndex + direction;
  if (next < 0 || next >= clips.length) return;

  selectedIndex = next;
  _applySelection();

  // 滚动到可见区域
  const el = _container.querySelector(`[data-id="${clips[selectedIndex].id}"]`);
  if (el) el.scrollIntoView({ block: "nearest", behavior: "smooth" });
}

/** 对当前选中条目执行"选择"（写入剪贴板并隐藏窗口）。 */
export async function confirmSelection() {
  if (selectedIndex < 0 || selectedIndex >= clips.length) return;
  const clip = clips[selectedIndex];
  try {
    await selectClip(clip.id);
  } catch (err) {
    console.error("selectClip 失败:", err);
  }
}

/** 关闭当前打开的操作菜单（如果有）。 */
export function closeOpenMenu() {
  if (_openMenu) {
    _openMenu.classList.add("hidden");
    _openMenu = null;
  }
}

// ── 内部辅助 ───────────────────────────────────────────────────────────────

function _clearList() {
  // 保留 empty-state 节点
  while (_container.firstChild) {
    _container.removeChild(_container.firstChild);
  }
  _container.appendChild(_emptyState);
  _updateEmptyState();
}

async function _loadPage() {
  if (loading || !hasMore) return;
  loading = true;

  // 显示加载指示器
  let spinner = _container.querySelector(".load-more-spinner");
  if (!spinner) {
    spinner = document.createElement("div");
    spinner.className = "load-more-spinner";
    spinner.textContent = "Loading…";
    _container.appendChild(spinner);
  }

  try {
    const page = await getClips(currentQuery, favoritesOnly, offset, PAGE_SIZE);
    if (page.length < PAGE_SIZE) hasMore = false;

    page.forEach((clip) => {
      clips.push(clip);
      const el = createClipElement(clip);
      _container.insertBefore(el, spinner);
    });

    offset += page.length;
  } catch (err) {
    console.error("getClips 失败:", err);
  } finally {
    loading = false;
    spinner.remove();
    _updateEmptyState();
  }
}

function _onScroll() {
  const { scrollTop, scrollHeight, clientHeight } = _container;
  if (scrollHeight - scrollTop - clientHeight < 100) {
    _loadPage();
  }
}

function _applySelection() {
  _container.querySelectorAll(".clip-item").forEach((el, i) => {
    el.classList.toggle("selected", i === selectedIndex);
    el.setAttribute("aria-selected", i === selectedIndex);
  });
}

function _updateEmptyState() {
  const hasItems = clips.length > 0;
  _emptyState.style.display = hasItems ? "none" : "";
}

// ── 条目 DOM 构建 ──────────────────────────────────────────────────────────

/**
 * 根据 ClipItem 数据构建完整的列表条目 DOM。
 * 所有用户内容均通过 textContent 写入（防 XSS）。
 * @param {ClipItem} clip
 * @returns {HTMLElement}
 */
export function createClipElement(clip) {
  const item = document.createElement("div");
  item.className = "clip-item";
  item.dataset.id = clip.id;
  item.setAttribute("role", "option");
  item.setAttribute("aria-selected", "false");

  // ── 缩略图（仅图片类型） ──
  if (clip.content_type === "image" && clip.image_data) {
    const thumb = document.createElement("img");
    thumb.className = "clip-thumbnail";
    thumb.alt = "image";
    // image_data 是字节数组，转换为 base64
    const b64 = _bytesToBase64(clip.image_data);
    thumb.src = `data:image/png;base64,${b64}`;
    item.appendChild(thumb);
  }

  // ── 内容区 ──
  const content = document.createElement("div");
  content.className = "clip-content";

  const preview = document.createElement("div");
  preview.className = "clip-preview";
  // 安全：textContent，绝不使用 innerHTML
  const displayText = _getDisplayText(clip);
  preview.textContent = displayText;
  content.appendChild(preview);

  // meta 行
  const meta = document.createElement("div");
  meta.className = "clip-meta";

  const typeIcon = document.createElement("span");
  typeIcon.className = "clip-type-icon";
  typeIcon.textContent = _typeIcon(clip.content_type);
  meta.appendChild(typeIcon);

  const timeEl = document.createElement("span");
  timeEl.className = "clip-time";
  timeEl.textContent = formatRelativeTime(clip.created_at);
  meta.appendChild(timeEl);

  const sizeEl = document.createElement("span");
  sizeEl.className = "clip-size";
  sizeEl.textContent = _formatSize(clip.byte_size);
  meta.appendChild(sizeEl);

  if (clip.is_favorite) {
    const favStar = document.createElement("span");
    favStar.className = "clip-fav-indicator";
    favStar.textContent = "★";
    favStar.title = "Favorited";
    meta.appendChild(favStar);
  }

  content.appendChild(meta);
  item.appendChild(content);

  // ── ⋯ 操作按钮 ──
  const actionsBtn = document.createElement("button");
  actionsBtn.className = "clip-actions-btn";
  actionsBtn.title = "More actions";
  actionsBtn.setAttribute("aria-label", "More actions");
  actionsBtn.textContent = "⋯";
  item.appendChild(actionsBtn);

  // ── 操作菜单 ──
  const menu = _createActionMenu(clip, item, actionsBtn);
  menu.classList.add("hidden");
  item.appendChild(menu);

  // 点击条目主体区域 → 选中（写入剪贴板）
  item.addEventListener("click", (e) => {
    if (e.target === actionsBtn || menu.contains(e.target)) return;
    // 更新 selectedIndex
    const allItems = Array.from(_container.querySelectorAll(".clip-item"));
    selectedIndex = allItems.indexOf(item);
    _applySelection();
    confirmSelection();
  });

  // 点击 ⋯ 按钮 → 切换菜单
  actionsBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    const isOpen = !menu.classList.contains("hidden");
    closeOpenMenu();
    if (!isOpen) {
      menu.classList.remove("hidden");
      _openMenu = menu;
    }
  });

  return item;
}

/**
 * 构建操作菜单（Favorite / Copy / Delete）。
 */
function _createActionMenu(clip, itemEl, actionsBtn) {
  const menu = document.createElement("div");
  menu.className = "action-menu";
  menu.setAttribute("role", "menu");

  // ── Favorite ──
  const favBtn = document.createElement("button");
  favBtn.className = "action-btn favorite-btn" + (clip.is_favorite ? " is-favorite" : "");
  favBtn.setAttribute("role", "menuitem");
  favBtn.textContent = clip.is_favorite ? "★  Unfavorite" : "☆  Favorite";
  favBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      const newState = await toggleFavorite(clip.id);
      clip.is_favorite = newState;
      // 更新按钮状态
      favBtn.textContent = newState ? "★  Unfavorite" : "☆  Favorite";
      favBtn.classList.toggle("is-favorite", newState);
      // 更新 meta 行里的收藏星
      const existingStar = itemEl.querySelector(".clip-fav-indicator");
      if (newState && !existingStar) {
        const star = document.createElement("span");
        star.className = "clip-fav-indicator";
        star.textContent = "★";
        star.title = "Favorited";
        itemEl.querySelector(".clip-meta").appendChild(star);
      } else if (!newState && existingStar) {
        existingStar.remove();
      }
    } catch (err) {
      console.error("toggleFavorite 失败:", err);
    }
    closeOpenMenu();
  });
  menu.appendChild(favBtn);

  // ── Copy ──
  const copyBtn = document.createElement("button");
  copyBtn.className = "action-btn";
  copyBtn.setAttribute("role", "menuitem");
  copyBtn.textContent = "⎘  Copy";
  copyBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      const text = _getDisplayText(clip);
      await navigator.clipboard.writeText(text);
    } catch (err) {
      console.error("复制失败:", err);
    }
    closeOpenMenu();
  });
  menu.appendChild(copyBtn);

  // ── Delete ──
  const delBtn = document.createElement("button");
  delBtn.className = "action-btn danger";
  delBtn.setAttribute("role", "menuitem");
  delBtn.textContent = "✕  Delete";
  delBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      await deleteClip(clip.id);
      removeClip(clip.id);
    } catch (err) {
      console.error("deleteClip 失败:", err);
    }
    closeOpenMenu();
  });
  menu.appendChild(delBtn);

  return menu;
}

// ── 格式化工具 ─────────────────────────────────────────────────────────────

/**
 * 将 Unix 时间戳（秒）转换为相对时间字符串。
 * @param {number} timestamp — Unix timestamp (seconds)
 * @returns {string}
 */
export function formatRelativeTime(timestamp) {
  const now  = Math.floor(Date.now() / 1000);
  const diff = now - timestamp;

  if (diff < 60)           return "just now";
  if (diff < 3600)         return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400)        return `${Math.floor(diff / 3600)} hr ago`;
  if (diff < 86400 * 2)    return "yesterday";
  return `${Math.floor(diff / 86400)} days ago`;
}

function _typeIcon(contentType) {
  switch (contentType) {
    case "image": return "🖼";
    case "html":  return "</>";
    default:      return "T";
  }
}

function _formatSize(bytes) {
  if (bytes < 1024)        return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function _getDisplayText(clip) {
  if (clip.content_type === "text" || clip.content_type === "html") {
    return clip.text_content || "";
  }
  if (clip.content_type === "image") {
    return "[Image]";
  }
  return "";
}

function _bytesToBase64(bytes) {
  let binary = "";
  const arr = new Uint8Array(bytes);
  for (let i = 0; i < arr.length; i++) {
    binary += String.fromCharCode(arr[i]);
  }
  return btoa(binary);
}
