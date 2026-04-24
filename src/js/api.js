/**
 * api.js — Tauri IPC 封装层
 * 这是唯一允许直接访问 window.__TAURI__ 的模块。
 */

// Tauri v2 JS API 入口
const { invoke } = window.__TAURI__.core;
const { listen }  = window.__TAURI__.event;

// ── IPC 命令 ────────────────────────────────────────────────────────────────

/**
 * 查询剪贴板条目列表。
 * @param {string|null} query        — 搜索关键词（null 表示不过滤）
 * @param {boolean}     favoritesOnly — 是否只返回收藏
 * @param {number}      offset        — 分页偏移
 * @param {number}      limit         — 每页条数
 * @returns {Promise<ClipItem[]>}
 */
export function getClips(query = null, favoritesOnly = false, offset = 0, limit = 20) {
  return invoke("get_clips", { query, favoritesOnly, offset, limit });
}

/**
 * 删除指定条目。
 * @param {number} id
 * @returns {Promise<void>}
 */
export function deleteClip(id) {
  return invoke("delete_clip", { id });
}

/**
 * 切换收藏状态。
 * @param {number} id
 * @returns {Promise<boolean>} — 切换后的新状态
 */
export function toggleFavorite(id) {
  return invoke("toggle_favorite", { id });
}

/**
 * 清空历史（保留收藏）。
 * @returns {Promise<void>}
 */
export function clearHistory() {
  return invoke("clear_history");
}

/**
 * 选中条目：写入系统剪贴板并隐藏窗口。
 * @param {number} id
 * @returns {Promise<void>}
 */
export function selectClip(id) {
  return invoke("select_clip", { id });
}

/**
 * 读取应用配置。
 * @returns {Promise<AppConfig>}
 */
export function getConfig() {
  return invoke("get_config");
}

/**
 * 保存应用配置。
 * @param {AppConfig} newConfig
 * @returns {Promise<void>}
 */
export function updateConfig(newConfig) {
  return invoke("update_config", { newConfig });
}

// ── 事件订阅 ─────────────────────────────────────────────────────────────────

/**
 * 订阅"新条目已添加"事件。
 * @param {function(ClipItem): void} callback
 * @returns {Promise<UnlistenFn>}
 */
export function onClipAdded(callback) {
  return listen("clip-added", (event) => callback(event.payload));
}

/**
 * 订阅"条目已删除"事件。
 * @param {function(number): void} callback — payload 为被删除条目的 id
 * @returns {Promise<UnlistenFn>}
 */
export function onClipRemoved(callback) {
  return listen("clip-removed", (event) => callback(event.payload));
}
