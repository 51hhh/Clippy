import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  PinCanvasProject,
  PinCanvasSaveMode,
  PinCanvasSaveResult,
  StructuredOcr,
  TranslationBatch,
  ViewerColor,
  ViewerHandle,
  ViewerPayload,
  ViewerReply,
  ViewerRequest,
  ViewerSettings,
  ViewerTextSource,
  ViewerTranslateOptions,
} from "../ipc-types.ts";
import { checkedStructuredOcr, parseImageCodeScanResponse } from "./validators.ts";
import type { ImageCodeScanResponse } from "./validators.ts";

function checkedViewerPayload(value: ViewerPayload): ViewerPayload {
  const source = value?.source;
  if (!value?.handle?.sessionId || !value.handle.snapshotId || !value.label?.startsWith("image-viewer-")
    || !source || source.mediaType !== "image/png" || !/^[a-f0-9]{64}$/i.test(source.contentHash)
    || !Number.isInteger(source.width) || !Number.isInteger(source.height)
    || source.width < 1 || source.height < 1 || source.width > 16384 || source.height > 16384
    || source.width * source.height > 32 * 1024 * 1024
    || !Number.isInteger(source.byteLength) || source.byteLength < 1 || source.byteLength > 64 * 1024 * 1024
    || typeof source.sensitive !== "boolean" || typeof value.limits?.canEdit !== "boolean"
    || typeof value.limits?.canScan !== "boolean") throw new Error("viewer.invalid_payload");
  return value;
}

/** 查看器 IPC 均携带实际快照和请求身份，不接受旧窗口的晚到回包。 */
const VIEWER_REQUEST_COMMANDS = [
  "recognize_viewer",
  "detect_viewer_codes",
  "translate_viewer",
  "sample_viewer_color",
  "copy_viewer_image",
  "save_viewer_image",
  "pin_viewer_image",
  "copy_viewer_text",
] as const;
type ViewerRequestCommand = typeof VIEWER_REQUEST_COMMANDS[number];

async function viewerInvoke<T>(command: ViewerRequestCommand, request: ViewerRequest, args: Record<string, unknown> = {}): Promise<ViewerReply<T>> {
  const reply = await invoke<ViewerReply<T>>(command, { request, ...args });
  if (reply?.sessionId !== request.sessionId || reply.snapshotId !== request.snapshotId || reply.requestId !== request.requestId
    || !Object.hasOwn(reply, "value")) throw new Error("viewer.stale_request");
  return reply;
}
export async function openImageViewer(id: number): Promise<ViewerPayload> {
  return checkedViewerPayload(await invoke<ViewerPayload>("open_image_viewer", { id }));
}
export async function getViewerPayload(): Promise<ViewerPayload> {
  return checkedViewerPayload(await invoke<ViewerPayload>("get_viewer_payload"));
}
export function getViewerSettings(): Promise<ViewerSettings> {
  return invoke("get_viewer_settings");
}
export function getViewerImageUrl(payload: ViewerPayload): string {
  // Tauri 会编码整个 filePath，必须在转换窗口段后再连接快照段。
  return `${convertFileSrc(payload.label, "viewer-frame")}/${encodeURIComponent(payload.handle.snapshotId)}`;
}
export function viewerReady(handle: ViewerHandle): Promise<void> { return invoke("viewer_ready", { handle }); }
export function closeImageViewer(handle: ViewerHandle): Promise<void> { return invoke("close_image_viewer", { handle }); }
export async function getViewerFullscreen(handle: ViewerHandle): Promise<boolean> {
  const value = await invoke<boolean>("get_viewer_fullscreen", { handle });
  if (typeof value !== "boolean") throw new Error("viewer.invalid_window_state");
  return value;
}
export function setViewerFullscreen(handle: ViewerHandle, fullscreen: boolean): Promise<void> {
  return invoke("set_viewer_fullscreen", { handle, fullscreen });
}
export function minimizeImageViewer(handle: ViewerHandle): Promise<void> { return invoke("minimize_image_viewer", { handle }); }
export function startViewerDrag(handle: ViewerHandle): Promise<void> { return invoke("start_viewer_drag", { handle }); }
/** 只订阅当前原生窗口；事件提示状态可能改变，实际全屏状态仍由专用IPC查询。 */
export async function onViewerWindowChanged(callback: () => void): Promise<UnlistenFn> {
  const current = getCurrentWindow();
  const listeners = await Promise.allSettled([
    current.onResized(() => callback()),
    current.onFocusChanged(event => { if (event.payload) callback(); }),
  ]);
  const disposers = listeners.flatMap(result => result.status === "fulfilled" ? [result.value] : []);
  const failure = listeners.find(result => result.status === "rejected");
  if (failure?.status === "rejected") { disposers.forEach(dispose => dispose()); throw failure.reason; }
  return () => disposers.forEach(dispose => dispose());
}
export async function recognizeViewer(request: ViewerRequest): Promise<ViewerReply<StructuredOcr>> {
  const reply = await viewerInvoke<StructuredOcr>("recognize_viewer", request);
  return { ...reply, value: checkedStructuredOcr(reply.value) };
}
export async function detectViewerCodes(request: ViewerRequest): Promise<ViewerReply<ImageCodeScanResponse>> {
  const reply = await viewerInvoke<unknown>("detect_viewer_codes", request);
  return { ...reply, value: parseImageCodeScanResponse(reply.value) };
}
export function translateViewer(request: ViewerRequest, options: ViewerTranslateOptions): Promise<ViewerReply<TranslationBatch>> {
  return viewerInvoke("translate_viewer", request, { options });
}
export async function sampleViewerColor(request: ViewerRequest, x: number, y: number): Promise<ViewerReply<ViewerColor>> {
  const reply = await viewerInvoke<ViewerColor>("sample_viewer_color", request, { x, y });
  const value = reply.value;
  if (value?.x !== x || value.y !== y || !Array.isArray(value.rgba) || value.rgba.length !== 4
    || value.rgba.some(channel => !Number.isInteger(channel) || channel < 0 || channel > 255)
    || value.hex !== "#" + value.rgba.slice(0, 3).map(channel => channel.toString(16).padStart(2, "0")).join("").toUpperCase()
    || typeof value.rgb !== "string") throw new Error("viewer.invalid_color");
  return reply;
}
export function copyViewerImage(request: ViewerRequest, document: PinCanvasProject | null): Promise<ViewerReply<null>> {
  return viewerInvoke("copy_viewer_image", request, { document });
}
export function saveViewerImage(request: ViewerRequest, mode: PinCanvasSaveMode, document: PinCanvasProject | null): Promise<ViewerReply<PinCanvasSaveResult>> {
  return viewerInvoke("save_viewer_image", request, { mode, document });
}
export function pinViewerImage(request: ViewerRequest, document: PinCanvasProject | null): Promise<ViewerReply<string>> {
  return viewerInvoke("pin_viewer_image", request, { document });
}
export function copyViewerText(request: ViewerRequest, source: ViewerTextSource, index: number): Promise<ViewerReply<null>> {
  return viewerInvoke("copy_viewer_text", request, { source, index });
}
