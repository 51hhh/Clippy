import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  CaptureAction,
  CaptureActionResult,
  CaptureDiagnosticsReport,
  CaptureLongshotHandoff,
  CaptureOrigin,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
  LongshotActivation,
  LongshotControllerOpenResult,
  LongshotHandle,
  LongshotOutputAction,
  LongshotOutputResult,
  LongshotSnapshot,
  PinCanvasProject,
  WindowProbeInstallOutcome,
  WindowProbeStatus,
} from "../ipc-types.ts";
import { parseImageCodeScanResponse } from "./validators.ts";
import type { ImageCodeScanResponse } from "./validators.ts";

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

/**
 * 只创建并隐藏独立长截图控制窗口；此时 ordinary 截图会话仍保持可用。
 * 调用者身份由后端的 WebviewWindow 注入，不允许前端提交 label。
 */
export function openLongshotController(
  selection: CaptureSelection,
): Promise<LongshotControllerOpenResult> {
  return invoke<LongshotControllerOpenResult>("open_longshot_controller", { selection });
}

/** 由存活的独立控制窗口发起 ordinary → longshot 交接。 */
export function activateLongshotController(): Promise<LongshotActivation> {
  return invoke<LongshotActivation>("activate_longshot_controller");
}

/** 控制页已经渲染 activation 结果，可以由后端显示并聚焦窗口。 */
export function markLongshotControllerReady(): Promise<void> {
  return invoke<void>("mark_longshot_controller_ready");
}

/**
 * 由已显示的独立控制窗口追加一次冻结选区。调用者身份仍由后端注入，
 * 前端只提交不可变的完整会话 handle，绝不提交窗口 label。
 */
export function appendLongshotController(handle: LongshotHandle): Promise<LongshotSnapshot> {
  return invoke<LongshotSnapshot>("append_longshot_controller", { handle });
}

/** 显式撤销二维画布最后一次提交；该操作不会重新捕获屏幕。 */
export function undoLongshotController(handle: LongshotHandle): Promise<LongshotSnapshot> {
  return invoke<LongshotSnapshot>("undo_longshot_controller", { handle });
}

/**
 * 获取 exact 长截图会话的画布 PNG 预览。二进制 IPC 结果仍是不可信输入：
 * 只接受当前 realm 的非空 ArrayBuffer，并把单次响应限制在 1 MiB 内。
 */
export async function previewLongshotController(handle: LongshotHandle): Promise<ArrayBuffer> {
  const value = await invoke<unknown>("preview_longshot_controller", { handle });
  try {
    if (!(value instanceof ArrayBuffer)) {
      throw new TypeError("invalid longshot preview response");
    }
    const byteLength = value.byteLength;
    if (byteLength <= 0 || byteLength > 1024 * 1024) {
      throw new TypeError("invalid longshot preview response");
    }
  } catch {
    throw new TypeError("invalid longshot preview response");
  }
  return value;
}

/**
 * 原子结束 exact 长截图会话，并在后端将最终 PNG 输出到指定目标。
 * 这个边界绝不传递窗口标签、PNG 或 base64；失败重试由后端保留 artifact。
 */
export function finishLongshotController(
  handle: LongshotHandle,
  action: LongshotOutputAction,
): Promise<LongshotOutputResult> {
  return invoke<LongshotOutputResult>("finish_longshot_controller", { handle, action });
}

/**
 * 关闭独立控制窗口；Active 会话必须携带完整 handle，尚未拿到 handle 时显式传 null。
 * 调用者窗口身份仍只由后端注入。
 */
export function cancelLongshotController(handle: LongshotHandle | null): Promise<void> {
  return invoke<void>("cancel_longshot_controller", { handle });
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

/** 重试已认领产物，调用窗口身份由后端注入。 */
export function retryCaptureAction(action: CaptureAction): Promise<CaptureActionResult> {
  return invoke<CaptureActionResult>("retry_capture_action", { action });
}

export function onCaptureLongshotHandoff(callback: (result: CaptureLongshotHandoff) => void): Promise<UnlistenFn> {
  return listen<CaptureLongshotHandoff>("capture-longshot-handoff", (event) => callback(event.payload));
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

/** 扫描当前截图会话的原始选区；不会访问剪贴历史或自动打开识别内容。 */
export async function scanCaptureSelection(
  selection: CaptureSelection,
): Promise<ImageCodeScanResponse> {
  const response = await invoke<unknown>("scan_capture_selection", { selection });
  return parseImageCodeScanResponse(response);
}
