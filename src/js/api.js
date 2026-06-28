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

export function setCodecVisible(visible) {
  return invoke("set_codec_visible", { visible });
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

/** 打开截图编辑器 */
export function showCaptureEditor() {
  return invoke("show_capture_editor");
}

/** 关闭当前窗口 */
export async function closeCurrentWindow() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow().close();
}

/** 读取待编辑截图 */
export function getPendingCapture() {
  return invoke("get_pending_capture");
}

/** 清理未消费的待编辑截图 */
export function clearPendingCapture() {
  return invoke("clear_pending_capture");
}

/** 复制截图编辑器导出的 PNG */
export function copyScreenshotImage(pngBase64) {
  return invoke("copy_screenshot_image", { pngBase64 });
}

/** 保存截图编辑器导出的 PNG */
export function saveScreenshotImage(pngBase64) {
  return invoke("save_screenshot_image", { pngBase64 });
}

/** 将截图编辑器导出的 PNG 贴到桌面 */
export function pinScreenshotImage(pngBase64) {
  return invoke("pin_screenshot_image", { pngBase64 });
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

/** 一键安装 tesseract-ocr（通过 pkexec 提权） */
export function ocrInstall() {
  return invoke("ocr_install");
}

/** 获取 URL 的 Open Graph 元数据（标题/描述/favicon），带后端缓存 */
export function fetchUrlMeta(url) {
  return invoke("fetch_url_meta", { url });
}

/** 获取剪贴板统计信息（总数/类型分布/存储大小等） */
export function getStats() {
  return invoke("get_stats");
}

/** 切换 tmux 缓冲区捕获 */
export function toggleTmuxCapture(enabled) {
  return invoke("toggle_tmux_capture", { enabled });
}

/** 检查 tmux 是否可用 */
export function tmuxAvailable() {
  return invoke("tmux_available");
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
