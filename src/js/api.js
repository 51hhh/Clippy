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

/** 按 id 获取图片数据（base64 编码的 PNG），仅 image 类型有值 */
export function getClipImage(id) {
  return invoke("get_clip_image", { id });
}

/** 按 id 获取完整条目（含 html_content），用于预览面板按需加载 */
export function getClipDetail(id) {
  return invoke("get_clip_detail", { id });
}

/** 切换预览面板可见性（同时调整窗口大小） */
export function setPreviewVisible(visible) {
  return invoke("set_preview_visible", { visible });
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

/** 检测安装类型：appimage（支持自动更新）/ deb（需手动下载） */
export function getInstallType() {
  return invoke("get_install_type");
}

/** 当前进程是否为 cargo target 开发产物（dev 模式下应禁用自启 toggle） */
export function isDevBinary() {
  return invoke("is_dev_binary");
}

/** 将条目钉到桌面 */
export function pinClip(id) {
  return invoke("pin_clip", { id });
}

/** 关闭贴图窗口 */
export function closePin(label) {
  return invoke("close_pin", { label });
}

/** 检查 OCR 是否可用（系统是否安装了 tesseract） */
export function ocrAvailable() {
  return invoke("ocr_available");
}

/** OCR 识别图片中的文字 */
export function ocrImage(id) {
  return invoke("ocr_image", { id });
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

export function onShortcutRegisterFailed(callback) {
  return listen("shortcut-register-failed", (event) => callback(event.payload));
}

export function onPinCurrent(callback) {
  return listen("pin-current", () => callback());
}

// ── 更新相关（懒加载，避免 settings 窗口引入 api.js 时因 plugin 未就绪而阻塞） ──

/** 获取应用版本号 */
export async function getAppVersion() {
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

/** 检查更新，返回 { available, version, body, update } 或 null */
export async function checkUpdate() {
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  return {
    available: true,
    version: update.version,
    body: update.body || "",
    update,
  };
}

/** 下载并安装更新，支持进度回调 onProgress({ total, received }) */
export async function downloadAndInstallUpdate(update, onProgress) {
  await update.downloadAndInstall((event) => {
    if (event.event === "Started" && onProgress) {
      onProgress({ total: event.data.contentLength || 0, received: 0 });
    } else if (event.event === "Progress" && onProgress) {
      onProgress({ chunkLength: event.data.chunkLength });
    }
  });
}

/** 打开外部 URL（用于 deb 回退下载） */
export async function openExternalUrl(url) {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  return openUrl(url);
}
