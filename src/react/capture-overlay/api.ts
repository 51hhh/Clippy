import {
  cancelCaptureOverlay,
  getCaptureOverlay,
  runCaptureAction,
  translateCaptureSelection,
} from "../../js/api.js";
import type {
  CaptureAction,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
} from "./types";

export const overlayApi = {
  get: (label: string) => getCaptureOverlay(label) as Promise<CaptureOverlayPayload>,
  cancel: (sessionId: string) => cancelCaptureOverlay(sessionId) as Promise<void>,
  run: (action: CaptureAction, selection: CaptureSelection) =>
    runCaptureAction(action, selection) as Promise<{
      action: CaptureAction;
      path: string | null;
      pinLabel: string | null;
    }>,
  translate: (selection: CaptureSelection) =>
    translateCaptureSelection(selection) as Promise<CaptureTranslationResult>,
};
