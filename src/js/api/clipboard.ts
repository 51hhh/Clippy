import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  ClipItem,
  PasteOutcome,
  PasteStatus,
  PlatformInfo,
  SpokenText,
  StructuredOcr,
  TranslationBatch,
  TranslationHistoryEntry,
  TranslationProvider,
  UrlMeta,
} from "../ipc-types.ts";
import { checkedStructuredOcr, parseImageCodeScanResponse } from "./validators.ts";
import type { ImageCodeScanResponse } from "./validators.ts";

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

/** 检查 OCR 是否可用（系统是否安装了 tesseract） */
export function ocrAvailable(): Promise<boolean> {
  return invoke<boolean>("ocr_available");
}

/** OCR 识别图片中的文字 */
export function ocrImage(id: number): Promise<string> {
  return invoke<string>("ocr_image", { id });
}

/** 主侧栏显示真实识别管线来源；旧 String API 保持兼容。 */
export async function ocrImageResult(id: number): Promise<StructuredOcr> {
  return checkedStructuredOcr(await invoke<StructuredOcr>("ocr_image_result", { id }));
}

/**
 * 显式扫描图片剪贴条目中的本地 QR Code / Code 39。
 *
 * 不会联网、不会持久化结果，也不会复制、打开或导航任何识别文本。
 */
export async function detectImageCodes(id: number): Promise<ImageCodeScanResponse> {
  const response = await invoke<unknown>("detect_image_codes", { id });
  return parseImageCodeScanResponse(response);
}

/** 获取 URL 的 Open Graph 元数据（标题/描述/favicon），带后端缓存 */
export function fetchUrlMeta(url: string): Promise<UrlMeta> {
  return invoke<UrlMeta>("fetch_url_meta", { url });
}

// -- 事件 --

export function onClipAdded(callback: (clip: ClipItem) => void): Promise<UnlistenFn> {
  return listen<ClipItem>("clip-added", (event) => callback(event.payload));
}

export function onClipRemoved(callback: (id: number) => void): Promise<UnlistenFn> {
  return listen<number>("clip-removed", (event) => callback(event.payload));
}
/** 自动粘贴受系统权限/会话限制时，剪贴板已写入但需要用户手动粘贴。 */
export function onPasteFallback(callback: (outcome: PasteOutcome) => void): Promise<UnlistenFn> {
  return listen<PasteOutcome>("paste-fallback", (event) => callback(event.payload));
}
/** 原生层即将显式隐藏主窗口；blur 不一定发生，前端必须据此释放大快照。 */
export function onMainWindowWillHide(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("main-window-will-hide", () => callback());
}
