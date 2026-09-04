/**
 * api.ts - Tauri IPC 封装层。
 * 唯一允许直接访问 Tauri invoke/listen API 的模块。
 */

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
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
  CaptureDiagnosticsReport,
  CaptureOrigin,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
  ClipboardStats,
  ClipItem,
  InstallType,
  PasteOutcome,
  PasteStatus,
  PlatformInfo,
  PinImageSharpened,
  PinCanvasProject,
  PinCanvasSaveMode,
  PinCanvasSaveResult,
  PinPayload,
  PinProject,
  PinToolbarBounds,
  PinState,
  PinUpdate,
  ShortcutConflict,
  ShortcutRegisterFailure,
  SpokenText,
  TranslationBatch,
  TranslationHistoryEntry,
  TranslationProvider,
  TranslationResult,
  UrlMeta,
  WindowProbeInstallOutcome,
  WindowProbeStatus,
} from "./ipc-types.ts";

export type {
  AppConfig,
  CaptureAction,
  CaptureActionResult,
  CaptureDiagnosticsReport,
  CaptureOrigin,
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
  PlatformCapabilities,
  PlatformCapability,
  PlatformInfo,
  PinImageSharpened,
  PinCanvasProject,
  PinCanvasSaveMode,
  PinCanvasSaveResult,
  PinPayload,
  PinProject,
  PinToolbarBounds,
  PinState,
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
  WindowProbeInstallOutcome,
  WindowProbeStatus,
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

/** 后端检测到的平台、桌面会话和能力边界。 */
export function getPlatformInfo(): Promise<PlatformInfo> {
  return invoke<PlatformInfo>("get_platform_info");
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

/**
 * 按 id 获取**原图**（base64 编码的 PNG），仅 image 类型有值。
 *
 * 列表行别用这个，用 `getClipThumbnail`：一张全屏截图是几 MB，行里那格只有 48 px。
 */
export function getClipImage(id: number): Promise<string | null> {
  return invoke<string | null>("get_clip_image", { id });
}

/**
 * 按 id 获取列表行用的缩略图（base64 编码的 PNG，最长边 128 px），仅 image 类型有值。
 *
 * 后端缩好再传：为了画 48 px 把整张原图送进 webview 再解码，一次开面板十几个图片条目
 * 就是几十 MB IPC 加十几次全尺寸 PNG 解码，全部落在 webview 那一个线程上。
 */
export function getClipThumbnail(id: number): Promise<string | null> {
  return invoke<string | null>("get_clip_thumbnail", { id });
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

/**
 * 冻结帧的原始像素：RGBA8、行优先、无 padding，尺寸取 payload 的 pixelWidth/pixelHeight。
 *
 * 后端用 `tauri::ipc::Response` 直接回二进制，所以这里拿到的是 ArrayBuffer 而不是字符串。
 * 像素曾经跟着 payload 走 JSON（pngBase64），代价是两头各一次编解码 —— 全屏帧实测占掉
 * 覆盖层出现前的一半时间。别改回字符串。
 */
export function getCaptureFrame(label: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("get_capture_frame", { label });
}

/** WebKit 原生资源管线使用的无损冻结帧 URL；convertFileSrc 会处理 Windows URL 形式。 */
export function getCaptureFrameUrl(label: string): string {
  return convertFileSrc(label, "capture-frame");
}

/**
 * 报告覆盖层已经画出第一帧，后端这才把窗口显示出来。
 * 覆盖层是隐藏建窗的：提前显示就会让用户看到一整屏 webview 默认底色（白屏）。
 *
 * 同时捎上**实测的可见视口**：后端算出来的显示器逻辑尺寸只有这里能被验证一次
 * （不变量 I4），对不上就说明几何算错了、界面正在错位。首帧画完意味着窗口已经布局完成，
 * 所以这是天然的时机，不必为它另开一个 IPC 命令。
 */
export function markCaptureOverlayReady(
  label: string,
  viewportWidth?: number,
  viewportHeight?: number,
): Promise<void> {
  return invoke<void>("mark_capture_overlay_ready", { label, viewportWidth, viewportHeight });
}

export function cancelCaptureOverlay(sessionId: string): Promise<void> {
  return invoke<void>("cancel_capture_overlay", { sessionId });
}

/** 提交选区与 renderer v2 操作层；权威 PNG 由后端从可信冻结帧生成。 */
export function commitCaptureAction(
  action: CaptureAction,
  selection: CaptureSelection,
  project: PinCanvasProject,
  origin: CaptureOrigin | null = null,
): Promise<CaptureActionResult> {
  return invoke<CaptureActionResult>("commit_capture_action", {
    action,
    selection,
    project,
    origin,
  });
}

/** 窗口速选依赖的 GNOME Shell 扩展的服务状态 */
export function getWindowProbeStatus(): Promise<WindowProbeStatus> {
  return invoke<WindowProbeStatus>("get_window_probe_status");
}

/**
 * 安装窗口速选扩展。只能由用户在设置页显式点击触发——往用户的 GNOME 里装扩展
 * 是很打扰的动作，应用不擅自代劳。
 */
export function installWindowProbeExtension(): Promise<WindowProbeInstallOutcome> {
  return invoke<WindowProbeInstallOutcome>("install_window_probe_extension");
}

export function uninstallWindowProbeExtension(): Promise<WindowProbeStatus> {
  return invoke<WindowProbeStatus>("uninstall_window_probe_extension");
}

/**
 * 采集截图几何诊断报告。约 0.5–1 秒（内含一次真实的舞台图请求，只读 PNG 头）。
 *
 * 报告不含截图像素也不含窗口标题，只写本机缓存目录；上传与否完全由用户决定。
 */
export function runCaptureDiagnostics(
  note: string | null = null,
): Promise<CaptureDiagnosticsReport> {
  return invoke<CaptureDiagnosticsReport>("run_capture_diagnostics", { note });
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

/**
 * 贴图显示 PNG 的原生资源 URL。revision=0 是首帧（补偿赶上就直接取补偿图，否则取原图），
 * 后台补偿晚到时事件给出更高版本号，避免 WebKit 复用已经解码过的首帧。
 */
export function getPinImageUrl(label: string, revision: number): string {
  return `${convertFileSrc(label, "pin-frame")}?revision=${revision}`;
}

/**
 * 贴图工具条能待的范围（窗口局部逻辑坐标）。
 *
 * 前端算不了这个：它只有 `window.innerWidth`，而贴图窗口的外框永远给工具条留够了位置，
 * 真正会超出屏幕的是窗口在屏幕上的位置——那要问合成器。宽或高为 0 表示查不到，
 * 调用方退回整个窗口。
 */
export function getPinToolbarBounds(label: string): Promise<PinToolbarBounds> {
  return invoke<PinToolbarBounds>("get_pin_toolbar_bounds", { label });
}

/** 贴图内容首帧加载完成后显示原生窗口 */
export function pinReady(label: string): Promise<void> {
  return invoke<void>("pin_ready", { label });
}

/** 更新贴图缩放、透明度或锁定状态 */
/** 应答只带可变字段，不带图片：调用方把它合并进手里的 payload（见 `PinState`） */
export function updatePin(label: string, update: PinUpdate): Promise<PinState> {
  return invoke<PinState>("update_pin", { label, update });
}

/** 复制贴图内容，不触发自动粘贴 */
export function copyPin(label: string): Promise<void> {
  return invoke<void>("copy_pin", { label });
}

export function savePin(label: string): Promise<string> {
  return invoke<string>("save_pin", { label });
}

/**
 * 贴图的**原图**（base64 PNG），画布导出的底图。
 *
 * 不能用 `getPinPayload` 给的那张：它优先是清晰度补偿版——按缓冲区分辨率渲染、
 * 并为"随后被合成器缩小"预先锐化过，单独看偏大且过冲。导出时单独取一次，
 * 用完即弃（导出是低频动作，不该让每个贴图窗口长期多驻一份原图）。
 */
export function getPinSourceImage(label: string): Promise<string | null> {
  return invoke<string | null>("get_pin_source_image", { label });
}

/**
 * 把贴图上画过的那一版存盘，可选同时进剪贴板。
 *
 * 普通来源条目由 `copyPin`/`savePin` 交付原图；工程来源条目交付保存时的 IDAT 预览。
 * renderer v2 的当前编辑结果只提交工程文档并让后端渲染；`pngBase64` 只保留给 v1 兼容路径。
 * 未修改的导入工程同时传两个 `null`，由后端复用权威 IDAT。
 */
export function savePinCanvas(
  label: string,
  pngBase64: string | null,
  toClipboard: boolean,
  mode: PinCanvasSaveMode,
  project: PinCanvasProject | null,
): Promise<PinCanvasSaveResult> {
  return invoke<PinCanvasSaveResult>("save_pin_canvas", { label, pngBase64, toClipboard, mode, project });
}

/** 由后端按固定 renderer v2 合成并写入剪贴板，不携带工程 iTXt。 */
export function copyPinCanvas(label: string, project: PinCanvasProject): Promise<void> {
  return invoke<void>("copy_pin_canvas", { label, pngBase64: null, project });
}

/**
 * 读一个 PNG 文件里的贴图工程数据。
 *
 * `null` = 这是张普通图片（没有工程块、块坏了、版本比当前应用新）。三种情况对用户都是
 * 同一件事：能看，不能继续编辑，所以不区分。
 */
export function readPinProject(path: string): Promise<PinProject | null> {
  return invoke<PinProject | null>("read_pin_project", { path });
}

/**
 * 后台算好的清晰版贴图到货了。
 *
 * 贴图先用原图上屏（开窗才不会被卡住），后端随后在别的线程上把它重新渲染成缓冲区
 * 分辨率并补偿掉合成器的缩小。事件只传资源版本号，PNG 由 WebKit 直接从 `pin-frame`
 * 取走，不再经 base64/JSON/JS 解码。见 `pin/resample.rs`。
 */
export function onPinImageSharpened(
  callback: (payload: PinImageSharpened) => void,
): Promise<UnlistenFn> {
  return listen<PinImageSharpened>("pin-image-sharpened", (event) => callback(event.payload));
}

/**
 * 用户又对同一个条目按了 Pin，而这张图已经贴出来了。
 *
 * 一个条目只对应一个贴图窗口（label 是 GNOME Shell 扩展的查找键，同名开两个会让第二张
 * 摆不了位）。后端因此只把既有窗口显示出来并发这个事件，由前端闪一下外围边框告诉用户
 * "它已经在这儿了"——不然那张贴图被压住或在别的工作区时，用户看到的就是"什么都没发生"。
 */
export function onPinAlreadyOpen(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("pin-already-open", () => callback());
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

/** 自动粘贴受系统权限/会话限制时，剪贴板已写入但需要用户手动粘贴。 */
export function onPasteFallback(callback: (outcome: PasteOutcome) => void): Promise<UnlistenFn> {
  return listen<PasteOutcome>("paste-fallback", (event) => callback(event.payload));
}

/** 已记录的快捷键注册失败。启动阶段的失败早于前端监听，只能主动查 */
export function getShortcutFailures(): Promise<ShortcutRegisterFailure[]> {
  return invoke<ShortcutRegisterFailure[]>("get_shortcut_failures");
}

export function onPinCurrent(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("pin-current", () => callback());
}

/** 原生层即将显式隐藏主窗口；blur 不一定发生，前端必须据此释放大快照。 */
export function onMainWindowWillHide(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("main-window-will-hide", () => callback());
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
