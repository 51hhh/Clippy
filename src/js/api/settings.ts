import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { DragDropEvent } from "@tauri-apps/api/window";
import {
  disable as disableAutostartPlugin,
  enable as enableAutostartPlugin,
  isEnabled as isAutostartEnabledPlugin,
} from "@tauri-apps/plugin-autostart";
import type {
  AppConfig,
  AppUpdateSnapshot,
  ArchiveExportResult,
  ArchiveImportSummary,
  ArchiveScope,
  ClipboardStats,
  InstallType,
  OcrHealthStatus,
  ShortcutConflict,
  ShortcutRegisterFailure,
  TranslationProvider,
} from "../ipc-types.ts";

/** 读取配置 */
export function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

/** 保存配置 */
export interface ShortcutUpdateOutcome { shortcut_status: "unchanged" | "applied" | "pending"; }
export function updateConfig(newConfig: AppConfig, options?: { requireShortcutsActive?: boolean }): Promise<ShortcutUpdateOutcome> {
  return invoke<ShortcutUpdateOutcome>("update_config", {
    newConfig,
    ...(options?.requireShortcutsActive ? { requireShortcutsActive: true } : {}),
  });
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
export function resumeShortcuts(): Promise<ShortcutUpdateOutcome> {
  return invoke<ShortcutUpdateOutcome>("resume_shortcuts");
}

/** 检测安装类型：appimage（支持自动更新）/ deb（需手动下载） */
export function getInstallType(): Promise<InstallType> {
  return invoke<InstallType>("get_install_type");
}

/** 当前进程是否为 cargo target 开发产物（dev 模式下应禁用自启 toggle） */
export function isDevBinary(): Promise<boolean> {
  return invoke<boolean>("is_dev_binary");
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

/** 设置窗口只在恢复快捷键成功后销毁。 */
export function closeSettings(): Promise<void> {
  return invoke<void>("close_settings");
}

/** 监听当前窗口的原生关闭请求。 */
export function onCurrentWindowCloseRequested(callback: () => void): Promise<UnlistenFn> {
  return getCurrentWindow().onCloseRequested((event) => {
    event.preventDefault();
    callback();
  });
}

/** 监听当前原生窗口的文件拖放事件，调用方负责在不再需要时卸载。 */
export function onCurrentWindowDragDrop(
  callback: (event: DragDropEvent) => void,
): Promise<UnlistenFn> {
  return getCurrentWindow().onDragDropEvent((event) => callback(event.payload));
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

/** 将条目钉到桌面 */
/** 一键安装 tesseract-ocr（通过 pkexec 提权） */
export function ocrInstall(): Promise<string> {
  return invoke<string>("ocr_install");
}

/** 使用识别路径本身的校验器检查增强 OCR 与 Tesseract fallback。 */
export function getOcrHealthStatus(manifestPath: string): Promise<OcrHealthStatus> {
  return invoke<OcrHealthStatus>("ocr_health_status", { manifestPath });
}

/** 选择增强 OCR manifest；取消返回 null，保存仍由设置页统一完成。 */
export function pickOcrManifest(): Promise<string | null> {
  return invoke<string | null>("pick_ocr_manifest");
}

/** 获取剪贴板统计信息（总数/类型分布/存储大小等） */
export function getStats(): Promise<ClipboardStats> {
  return invoke<ClipboardStats>("get_stats");
}

/** 将历史记录或 Pin 工作区导出为经过校验的本地 Clippy 归档。取消对话框返回 null。 */
export function exportClippyArchive(
  scope: ArchiveScope,
  includeSensitive: boolean,
): Promise<ArchiveExportResult | null> {
  return invoke<ArchiveExportResult | null>("export_clippy_archive", {
    scope,
    includeSensitive,
  });
}

/** 校验并事务导入 Clippy 归档。取消对话框返回 null。 */
export function importClippyArchive(): Promise<ArchiveImportSummary | null> {
  return invoke<ArchiveImportSummary | null>("import_clippy_archive");
}

/** 切换 tmux 缓冲区捕获 */
export function toggleTmuxCapture(enabled: boolean): Promise<void> {
  return invoke<void>("toggle_tmux_capture", { enabled });
}

/** 检查 tmux 是否可用 */
export function tmuxAvailable(): Promise<boolean> {
  return invoke<boolean>("tmux_available");
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

// -- 更新相关：应用进程持有任务，窗口通过 IPC 读取与订阅状态 --

/** 获取应用版本号 */
export async function getAppVersion(): Promise<string> {
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

/** 检查更新 */
export function checkUpdate(): Promise<AppUpdateSnapshot> {
  return invoke<AppUpdateSnapshot>("check_app_update");
}

/** 立即返回已接纳的状态；任务属于应用进程，不属于发起窗口。 */
export function downloadAndInstallUpdate(version: string): Promise<AppUpdateSnapshot> {
  return invoke<AppUpdateSnapshot>("install_app_update", { version });
}

export function getAppUpdateState(): Promise<AppUpdateSnapshot> {
  return invoke<AppUpdateSnapshot>("get_app_update_state");
}

export function onAppUpdateState(callback: (snapshot: AppUpdateSnapshot) => void): Promise<UnlistenFn> {
  return listen<AppUpdateSnapshot>("app-update-state", event => callback(event.payload));
}

/** 更新安装完成后，用户明确点击按钮才请求重启。 */
export function restartApp(): Promise<void> {
  return invoke<void>("restart_app");
}

/** 打开外部 URL（用于 deb 回退下载） */
export async function openExternalUrl(url: string): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  return openUrl(url);
}
