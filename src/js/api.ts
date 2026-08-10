/**
 * api.ts - Tauri IPC 封装层。
 * 唯一允许直接访问 Tauri invoke/listen API 的模块。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { DownloadEvent, Update } from "@tauri-apps/plugin-updater";
import type {
  AppConfig,
  CaptureAction,
  CaptureActionResult,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
  CapturedScreenshot,
  ClipboardStats,
  ClipItem,
  InstallType,
  PasteOutcome,
  PasteStatus,
  PinPayload,
  PinUpdate,
  TranslationProvider,
  TranslationResult,
  UrlMeta,
} from "./ipc-types.ts";

export type {
  AppConfig,
  CaptureAction,
  CaptureActionResult,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
  CapturedScreenshot,
  ClipboardStats,
  ClipItem,
  ContentType,
  InstallType,
  PasteBackend,
  PasteOutcome,
  PastePhase,
  PasteStatus,
  PinPayload,
  PinUpdate,
  TranslationProvider,
  TranslationResult,
  UrlMeta,
  WindowCandidate,
} from "./ipc-types.ts";

/** 剪贴板列表 */
export function getClips(
  query: string | null = null,
  favoritesOnly = false,
  offset = 0,
  limit = 20,
): Promise<ClipItem[]> {
  return invoke<ClipItem[]>("get_clips", { query, favoritesOnly, offset, limit });
}

/** 删除条目 */
export function deleteClip(id: number): Promise<void> {
  return invoke<void>("delete_clip", { id });
}

/** 切换收藏 */
export function toggleFavorite(id: number): Promise<boolean> {
  return invoke<boolean>("toggle_favorite", { id });
}

/** 清空历史 */
export function clearHistory(): Promise<void> {
  return invoke<void>("clear_history");
}

/** 选中条目并写入系统剪贴板 */
export function selectClip(id: number): Promise<PasteOutcome> {
  return invoke<PasteOutcome>("select_clip", { id });
}

/** 仅写入系统剪贴板，不隐藏窗口或模拟按键 */
export function copyClip(id: number): Promise<void> {
  return invoke<void>("copy_clip", { id });
}

/** 查询当前自动粘贴后端和授权状态 */
export function getPasteStatus(): Promise<PasteStatus> {
  return invoke<PasteStatus>("get_paste_status");
}

/** 显式请求 Wayland RemoteDesktop Portal 键盘控制权限 */
export function requestPastePermission(): Promise<PasteStatus> {
  return invoke<PasteStatus>("request_paste_permission");
}

/** 按 id 获取图片数据（base64 编码的 PNG），仅 image 类型有值 */
export function getClipImage(id: number): Promise<string | null> {
  return invoke<string | null>("get_clip_image", { id });
}

/** 按 id 获取完整条目（含 html_content），用于预览面板按需加载 */
export function getClipDetail(id: number): Promise<ClipItem> {
  return invoke<ClipItem>("get_clip_detail", { id });
}

/** 切换预览面板可见性（同时调整窗口大小） */
export function setPreviewVisible(visible: boolean): Promise<void> {
  return invoke<void>("set_preview_visible", { visible });
}

export function setCodecVisible(visible: boolean): Promise<void> {
  return invoke<void>("set_codec_visible", { visible });
}

/** 读取配置 */
export function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

/** 保存配置 */
export function updateConfig(newConfig: AppConfig): Promise<void> {
  return invoke<void>("update_config", { newConfig });
}

/** 显式翻译文本；语言与 request-id 由后端按当前配置分配 */
export function translateText(text: string): Promise<TranslationResult> {
  return invoke<TranslationResult>("translate_text", {
    text,
    sourceLanguage: null,
    targetLanguage: null,
    requestId: null,
  });
}

/** 显式翻译剪贴板条目；图片由后端先在本地执行 OCR */
export function translateClip(id: number): Promise<TranslationResult> {
  return invoke<TranslationResult>("translate_clip", {
    id,
    sourceLanguage: null,
    targetLanguage: null,
    requestId: null,
  });
}

/** 将指定翻译服务的 API key 写入系统 Secret Service */
export function setTranslationApiKey(
  provider: TranslationProvider,
  apiKey: string,
): Promise<void> {
  return invoke<void>("set_translation_api_key", { provider, apiKey });
}

/** 查询指定翻译服务是否已保存 API key，不读取或回显密钥 */
export function hasTranslationApiKey(provider: TranslationProvider): Promise<boolean> {
  return invoke<boolean>("has_translation_api_key", { provider });
}

/** 从系统 Secret Service 删除指定翻译服务的 API key */
export function deleteTranslationApiKey(provider: TranslationProvider): Promise<void> {
  return invoke<void>("delete_translation_api_key", { provider });
}

/** 更新全局快捷键 */
export function updateShortcut(newShortcut: string): Promise<void> {
  return invoke<void>("update_shortcut", { newShortcut });
}

/** 检查快捷键冲突 */
export function checkShortcutConflict(shortcut: string): Promise<boolean> {
  return invoke<boolean>("check_shortcut_conflict", { shortcut });
}

/** 暂停全局快捷键 */
export function pauseShortcuts(): Promise<void> {
  return invoke<void>("pause_shortcuts");
}

/** 恢复全局快捷键 */
export function resumeShortcuts(): Promise<void> {
  return invoke<void>("resume_shortcuts");
}

/** 检测安装类型：appimage（支持自动更新）/ deb（需手动下载） */
export function getInstallType(): Promise<InstallType> {
  return invoke<InstallType>("get_install_type");
}

/** 当前进程是否为 cargo target 开发产物（dev 模式下应禁用自启 toggle） */
export function isDevBinary(): Promise<boolean> {
  return invoke<boolean>("is_dev_binary");
}

/** 打开截图编辑器 */
export function showCaptureEditor(): Promise<void> {
  return invoke<void>("show_capture_editor");
}

/** 启动冻结屏幕选区覆盖层 */
export function showCaptureOverlay(): Promise<void> {
  return invoke<void>("show_capture_overlay");
}

export function getCaptureOverlay(label: string): Promise<CaptureOverlayPayload> {
  return invoke<CaptureOverlayPayload>("get_capture_overlay", { label });
}

export function cancelCaptureOverlay(sessionId: string): Promise<void> {
  return invoke<void>("cancel_capture_overlay", { sessionId });
}

export function runCaptureAction(
  action: CaptureAction,
  selection: CaptureSelection,
): Promise<CaptureActionResult> {
  return invoke<CaptureActionResult>("run_capture_action", { action, selection });
}

/** 截图选区先在后端本地 OCR，再仅发送识别文本进行翻译。 */
export function translateCaptureSelection(
  selection: CaptureSelection,
): Promise<CaptureTranslationResult> {
  return invoke<CaptureTranslationResult>("translate_capture_selection", {
    selection,
    sourceLanguage: null,
    targetLanguage: null,
    requestId: null,
  });
}

/** 关闭当前窗口 */
export async function closeCurrentWindow(): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow().close();
}

/** 读取待编辑截图 */
export function getPendingCapture(): Promise<CapturedScreenshot> {
  return invoke<CapturedScreenshot>("get_pending_capture");
}

/** 清理未消费的待编辑截图 */
export function clearPendingCapture(): Promise<void> {
  return invoke<void>("clear_pending_capture");
}

/** 复制截图编辑器导出的 PNG */
export function copyScreenshotImage(pngBase64: string): Promise<void> {
  return invoke<void>("copy_screenshot_image", { pngBase64 });
}

/** 保存截图编辑器导出的 PNG */
export function saveScreenshotImage(pngBase64: string): Promise<string> {
  return invoke<string>("save_screenshot_image", { pngBase64 });
}

/** 将截图编辑器导出的 PNG 贴到桌面 */
export function pinScreenshotImage(pngBase64: string): Promise<string> {
  return invoke<string>("pin_screenshot_image", { pngBase64 });
}

/** 将条目钉到桌面 */
export function pinClip(id: number): Promise<string> {
  return invoke<string>("pin_clip", { id });
}

/** 关闭贴图窗口 */
export function closePin(label: string): Promise<void> {
  return invoke<void>("close_pin", { label });
}

/** 获取统一贴图渲染与交互状态 */
export function getPinPayload(label: string): Promise<PinPayload> {
  return invoke<PinPayload>("get_pin_payload", { label });
}

/** 贴图内容首帧加载完成后显示原生窗口 */
export function pinReady(label: string): Promise<void> {
  return invoke<void>("pin_ready", { label });
}

/** 更新贴图缩放、透明度或锁定状态 */
export function updatePin(label: string, update: PinUpdate): Promise<PinPayload> {
  return invoke<PinPayload>("update_pin", { label, update });
}

/** 复制贴图内容，不触发自动粘贴 */
export function copyPin(label: string): Promise<void> {
  return invoke<void>("copy_pin", { label });
}

export function savePin(label: string): Promise<string> {
  return invoke<string>("save_pin", { label });
}

export function editPin(label: string): Promise<void> {
  return invoke<void>("edit_pin", { label });
}

/** 检查 OCR 是否可用（系统是否安装了 tesseract） */
export function ocrAvailable(): Promise<boolean> {
  return invoke<boolean>("ocr_available");
}

/** OCR 识别图片中的文字 */
export function ocrImage(id: number): Promise<string> {
  return invoke<string>("ocr_image", { id });
}

/** 一键安装 tesseract-ocr（通过 pkexec 提权） */
export function ocrInstall(): Promise<string> {
  return invoke<string>("ocr_install");
}

/** 获取 URL 的 Open Graph 元数据（标题/描述/favicon），带后端缓存 */
export function fetchUrlMeta(url: string): Promise<UrlMeta> {
  return invoke<UrlMeta>("fetch_url_meta", { url });
}

/** 获取剪贴板统计信息（总数/类型分布/存储大小等） */
export function getStats(): Promise<ClipboardStats> {
  return invoke<ClipboardStats>("get_stats");
}

/** 切换 tmux 缓冲区捕获 */
export function toggleTmuxCapture(enabled: boolean): Promise<void> {
  return invoke<void>("toggle_tmux_capture", { enabled });
}

/** 检查 tmux 是否可用 */
export function tmuxAvailable(): Promise<boolean> {
  return invoke<boolean>("tmux_available");
}

// -- 事件 --

export function onClipAdded(callback: (clip: ClipItem) => void): Promise<UnlistenFn> {
  return listen<ClipItem>("clip-added", (event) => callback(event.payload));
}

export function onClipRemoved(callback: (id: number) => void): Promise<UnlistenFn> {
  return listen<number>("clip-removed", (event) => callback(event.payload));
}

export function onConfigChanged(callback: (config: AppConfig) => void): Promise<UnlistenFn> {
  return listen<AppConfig>("config-changed", (event) => callback(event.payload));
}

export function onShortcutRegisterFailed(
  callback: (shortcut: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("shortcut-register-failed", (event) => callback(event.payload));
}

export function onPinCurrent(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("pin-current", () => callback());
}

export function onCaptureLoaded(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("capture-loaded", () => callback());
}

// -- 更新相关（懒加载，避免 settings 窗口因 plugin 未就绪而阻塞） --

export interface AvailableUpdate {
  available: true;
  version: string;
  body: string;
  update: Update;
}

export type UpdateProgress =
  | { total: number; received: number }
  | { chunkLength: number };

/** 获取应用版本号 */
export async function getAppVersion(): Promise<string> {
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

/** 检查更新 */
export async function checkUpdate(): Promise<AvailableUpdate | null> {
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

/** 下载并安装更新 */
export async function downloadAndInstallUpdate(
  update: Update,
  onProgress?: (progress: UpdateProgress) => void,
): Promise<void> {
  await update.downloadAndInstall((event: DownloadEvent) => {
    if (event.event === "Started" && onProgress) {
      onProgress({ total: event.data.contentLength || 0, received: 0 });
    } else if (event.event === "Progress" && onProgress) {
      onProgress({ chunkLength: event.data.chunkLength });
    }
  });
}

/** 打开外部 URL（用于 deb 回退下载） */
export async function openExternalUrl(url: string): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  return openUrl(url);
}
