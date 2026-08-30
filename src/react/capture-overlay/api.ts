import {
  cancelCaptureOverlay,
  commitCaptureAction,
  copyText,
  getCaptureOverlay,
  translateCaptureSelection,
} from "../../js/api.ts";
import type {
  CaptureAction,
  CaptureActionResult,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
} from "../../js/ipc-types.ts";

export const overlayApi = {
  get: (label: string): Promise<CaptureOverlayPayload> => getCaptureOverlay(label),
  cancel: (sessionId: string): Promise<void> => cancelCaptureOverlay(sessionId),
  /** 提交画布渲染好的 PNG：裁剪与标注都已经合成进去，后端只负责落地。 */
  commit: (
    action: CaptureAction,
    sessionId: string,
    pngBase64: string,
  ): Promise<CaptureActionResult> => commitCaptureAction(action, sessionId, pngBase64),
  /** 选区翻译仍走后端裁剪：OCR 要的是原始像素，不是带标注的画布。 */
  translate: (selection: CaptureSelection): Promise<CaptureTranslationResult> =>
    translateCaptureSelection(selection),
  copyText: (text: string): Promise<void> => copyText(text),
};
