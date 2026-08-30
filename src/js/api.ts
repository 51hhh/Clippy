/**
 * api.ts - Tauri IPC 封装层。
 * 唯一允许直接访问 Tauri invoke/listen API 的模块。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  disable as disableAutostartPlugin,
  enable as enableAutostartPlugin,
  isEnabled as isAutostartEnabledPlugin,
} from "@tauri-apps/plugin-autostart";
import type { DownloadEvent, Update } from "@tauri-apps/plugin-updater";
import type {
  AppConfig,
  CaptureAction,
  CaptureActionResult,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
  ClipboardStats,
  ClipItem,
  InstallType,
  PasteOutcome,
  PasteStatus,
  PinPayload,
  PinUpdate,
  ShortcutConflict,
  ShortcutRegisterFailure,
  SpokenText,
  TranslationBatch,
  TranslationHistoryEntry,
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
  ServiceTranslation,
  ShortcutConflict,
  ShortcutRegisterFailure,
  SpokenText,
  TranslationBatch,
  TranslationHistoryEntry,
  TranslationProvider,
  TranslationResult,
  TranslationServiceConfig,
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

/** 仅复制用户明确请求的文本，不新增历史条目或触发自动粘贴。 */
export function copyText(text: string): Promise<void> {
  return invoke<void>("copy_text", { text });
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

/**
 * 显式翻译文本；语言与 request-id 由后端按当前配置分配。
 * `providers` 省略表示所有启用的服务，传单个服务即为该服务的重试。
 */
export function translateText(
  text: string,
  providers?: TranslationProvider[],
): Promise<TranslationBatch> {
  return invoke<TranslationBatch>("translate_text", {
    text,
    sourceLanguage: null,
    targetLanguage: null,
    requestId: null,
    providers: providers ?? null,
  });
}

/** 显式翻译剪贴板条目；图片由后端先在本地执行 OCR */
export function translateClip(
  id: number,
  providers?: TranslationProvider[],
): Promise<TranslationBatch> {
  return invoke<TranslationBatch>("translate_clip", {
    id,
    sourceLanguage: null,
    targetLanguage: null,
    requestId: null,
    providers: providers ?? null,
  });
}

/** 朗读一段文本（结果卡上的译文）。音频由后端取回，webview 不请求第三方主机 */
export function speakText(text: string, language?: string): Promise<SpokenText> {
  return invoke<SpokenText>("speak_text", { text, language: language ?? null });
}

/** 朗读剪贴板条目自身的文本；敏感条目由后端拒绝 */
export function speakClip(id: number, language?: string): Promise<SpokenText> {
  return invoke<SpokenText>("speak_clip", { id, language: language ?? null });
}

/** 已保存的翻译记录，最新的在前。`clipId` 省略表示不限条目 */
export function translationHistory(
  clipId?: number,
  limit?: number,
): Promise<TranslationHistoryEntry[]> {
  return invoke<TranslationHistoryEntry[]>("translation_history", {
    clipId: clipId ?? null,
    limit: limit ?? null,
  });
}

/** 清空全部翻译记录。译文落盘后用户必须有办法把它删掉 */
export function clearTranslationHistory(): Promise<void> {
  return invoke<void>("clear_translation_history");
}

/**
 * 将指定翻译服务的凭据写入系统 Secret Service。
 * `apiSecret` 只有双字段服务（有道 appSecret）需要，其余服务省略。
 */
export function setTranslationApiKey(
  provider: TranslationProvider,
  apiKey: string,
  apiSecret?: string,
): Promise<void> {
  return invoke<void>("set_translation_api_key", {
    provider,
    apiKey,
    apiSecret: apiSecret ?? null,
  });
}

/** 查询指定翻译服务的凭据是否已完整保存，不读取或回显密钥 */
export function hasTranslationApiKey(provider: TranslationProvider): Promise<boolean> {
  return invoke<boolean>("has_translation_api_key", { provider });
}

/** 从系统 Secret Service 删除指定翻译服务的全部凭据字段 */
export function deleteTranslationApiKey(provider: TranslationProvider): Promise<void> {
  return invoke<void>("delete_translation_api_key", { provider });
}

/** 检查快捷键是否已被桌面或本应用占用 */
export function checkShortcutConflict(shortcut: string): Promise<ShortcutConflict> {
  return invoke<ShortcutConflict>("check_shortcut_conflict", { shortcut });
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

/**
 * 提交覆盖层里已经裁剪并标注好的 PNG。
 * 后端不再自己裁一遍，否则画布上的标注会被丢掉。
 */
export function commitCaptureAction(
  action: CaptureAction,
  sessionId: string,
  pngBase64: string,
): Promise<CaptureActionResult> {
  return invoke<CaptureActionResult>("commit_capture_action", {
    action,
    sessionId,
    pngBase64,
  });
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

/** 当前 Webview 窗口标签。 */
export function getCurrentWindowLabel(): string {
  return getCurrentWindow().label;
}

/** 隐藏当前 Webview 窗口。 */
export function hideCurrentWindow(): Promise<void> {
  return getCurrentWindow().hide();
}

/** 启动当前 Webview 窗口的原生拖动。 */
export function startDraggingCurrentWindow(): Promise<void> {
  return getCurrentWindow().startDragging();
}

/** 关闭当前 Webview 窗口。 */
export function closeCurrentWindow(): Promise<void> {
  return getCurrentWindow().close();
}

/** 监听当前窗口的原生关闭请求。 */
export function onCurrentWindowCloseRequested(callback: () => void): Promise<UnlistenFn> {
  return getCurrentWindow().onCloseRequested((event) => {
    event.preventDefault();
    callback();
  });
}

/** 开启登录时自动启动。 */
export function enableAutostart(): Promise<void> {
  return enableAutostartPlugin();
}

/** 关闭登录时自动启动。 */
export function disableAutostart(): Promise<void> {
  return disableAutostartPlugin();
}

/** 查询登录时自动启动状态。 */
export function isAutostartEnabled(): Promise<boolean> {
  return isAutostartEnabledPlugin();
}

/** 选择截图保存目录，用户取消时返回 null */
export function pickScreenshotDirectory(): Promise<string | null> {
  return invoke<string | null>("pick_screenshot_directory");
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
  callback: (failure: ShortcutRegisterFailure) => void,
): Promise<UnlistenFn> {
  return listen<ShortcutRegisterFailure>("shortcut-register-failed", (event) =>
    callback(event.payload));
}

/** 已记录的快捷键注册失败。启动阶段的失败早于前端监听，只能主动查 */
export function getShortcutFailures(): Promise<ShortcutRegisterFailure[]> {
  return invoke<ShortcutRegisterFailure[]>("get_shortcut_failures");
}

export function onPinCurrent(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("pin-current", () => callback());
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
