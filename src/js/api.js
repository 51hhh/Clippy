/**
 * api.js — Tauri IPC 封装层
 * 唯一允许直接访问 Tauri API 的模块。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** 剪贴板列表 */
export function getClips(query = null, favoritesOnly = false, offset = 0, limit = 20) {
  return invoke("get_clips", { query, favoritesOnly, offset, limit });
}

/** 删除条目 */
export function deleteClip(id) {
  return invoke("delete_clip", { id });
}

/** 切换收藏 */
export function toggleFavorite(id) {
  return invoke("toggle_favorite", { id });
}

/** 清空历史 */
export function clearHistory() {
  return invoke("clear_history");
}

/** 选中条目并写入系统剪贴板 */
export function selectClip(id) {
  return invoke("select_clip", { id });
}

/** 读取配置 */
export function getConfig() {
  return invoke("get_config");
}

/** 保存配置 */
export function updateConfig(newConfig) {
  return invoke("update_config", { newConfig });
}

/** 更新全局快捷键 */
export function updateShortcut(newShortcut) {
  return invoke("update_shortcut", { newShortcut });
}

/** 检查快捷键冲突 */
export function checkShortcutConflict(shortcut) {
  return invoke("check_shortcut_conflict", { shortcut });
}

/** 暂停全局快捷键 */
export function pauseShortcuts() {
  return invoke("pause_shortcuts");
}

/** 恢复全局快捷键 */
export function resumeShortcuts() {
  return invoke("resume_shortcuts");
}

// ── 事件 ──

export function onClipAdded(callback) {
  return listen("clip-added", (event) => callback(event.payload));
}

export function onClipRemoved(callback) {
  return listen("clip-removed", (event) => callback(event.payload));
}

export function onConfigChanged(callback) {
  return listen("config-changed", (event) => callback(event.payload));
}
