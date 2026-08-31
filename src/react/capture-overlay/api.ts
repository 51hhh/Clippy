import {
  cancelCaptureOverlay,
  commitCaptureAction,
  copyText,
  getCaptureFrame,
  getCaptureOverlay,
  markCaptureOverlayReady,
  translateCaptureSelection,
} from "../../js/api.ts";
import type {
  CaptureAction,
  CaptureActionResult,
  CaptureOrigin,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
} from "../../js/ipc-types.ts";

export const overlayApi = {
  get: (label: string): Promise<CaptureOverlayPayload> => getCaptureOverlay(label),
  /** 冻结帧像素（原始 RGBA，二进制 IPC）。payload 里没有图，底图从这里来。 */
  frame: (label: string): Promise<ArrayBuffer> => getCaptureFrame(label),
  /** 首帧已画好 / 或已经有错误可以显示——两种情况都该让窗口露出来。 */
  ready: (label: string): Promise<void> => markCaptureOverlayReady(label),
  cancel: (sessionId: string): Promise<void> => cancelCaptureOverlay(sessionId),
  /**
   * 提交画布渲染好的 PNG：裁剪与标注都已经合成进去，后端只负责落地。
   * `origin` 是选区在桌面逻辑坐标里的位置，贴图靠它回到原处。
   */
  commit: (
    action: CaptureAction,
    sessionId: string,
    pngBase64: string,
    origin: CaptureOrigin | null,
  ): Promise<CaptureActionResult> => commitCaptureAction(action, sessionId, pngBase64, origin),
  /** 选区翻译仍走后端裁剪：OCR 要的是原始像素，不是带标注的画布。 */
  translate: (selection: CaptureSelection): Promise<CaptureTranslationResult> =>
    translateCaptureSelection(selection),
  copyText: (text: string): Promise<void> => copyText(text),
};
