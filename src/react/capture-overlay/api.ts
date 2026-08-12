import {
  cancelCaptureOverlay,
  copyText,
  getCaptureOverlay,
  runCaptureAction,
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
  run: (action: CaptureAction, selection: CaptureSelection): Promise<CaptureActionResult> =>
    runCaptureAction(action, selection),
  translate: (selection: CaptureSelection): Promise<CaptureTranslationResult> =>
    translateCaptureSelection(selection),
  copyText: (text: string): Promise<void> => copyText(text),
};
