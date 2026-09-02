import {
  closePin,
  copyPin,
  copyPinCanvas,
  getPinPayload,
  getPinSourceImage,
  getPinToolbarBounds,
  onPinAlreadyOpen,
  onPinImageSharpened,
  pinReady,
  savePin,
  savePinCanvas,
  updatePin,
} from "../../js/api.ts";
import type {
  PinCanvasProject,
  PinCanvasSaveMode,
  PinCanvasSaveResult,
  PinImageSharpened,
  PinPayload,
  PinState,
  PinToolbarBounds,
  PinUpdate,
} from "./types";

export const pinApi = {
  get: (label: string): Promise<PinPayload> => getPinPayload(label),
  ready: (label: string): Promise<void> => pinReady(label),
  /** 工具条能待的范围（窗口局部逻辑坐标）。见 `usePinToolbarBounds`。 */
  toolbarBounds: (label: string): Promise<PinToolbarBounds> => getPinToolbarBounds(label),
  update: (label: string, update: PinUpdate): Promise<PinState> => updatePin(label, update),
  copy: (label: string): Promise<void> => copyPin(label),
  copyCanvas: (label: string, pngBase64: string): Promise<void> => copyPinCanvas(label, pngBase64),
  save: (label: string): Promise<string> => savePin(label),
  /** 画布导出用的原图（base64 PNG）。见 `getPinSourceImage`。 */
  sourceImage: (label: string): Promise<string | null> => getPinSourceImage(label),
  /** 存下贴图上画过的那一版（`toClipboard` 为真时同时进剪贴板）。 */
  saveCanvas: (
    label: string,
    pngBase64: string,
    toClipboard: boolean,
    mode: PinCanvasSaveMode,
    project: PinCanvasProject | null,
  ): Promise<PinCanvasSaveResult> => savePinCanvas(label, pngBase64, toClipboard, mode, project),
  close: (label: string): Promise<void> => closePin(label),
  /** 订阅后台算好的清晰版图片（见 `rendering.ts` 与 `pin/resample.rs`）。 */
  onSharpened: (callback: (payload: PinImageSharpened) => void): Promise<() => void> =>
    onPinImageSharpened(callback),
  /** 订阅"这张图已经贴出来了"，用来闪一下外围边框提醒用户。 */
  onAlreadyOpen: (callback: () => void): Promise<() => void> => onPinAlreadyOpen(callback),
};
