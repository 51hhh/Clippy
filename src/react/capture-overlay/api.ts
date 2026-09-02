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
  PinCanvasProject,
} from "../../js/ipc-types.ts";

export const overlayApi = {
  get: (label: string): Promise<CaptureOverlayPayload> => getCaptureOverlay(label),
  /** 冻结帧像素（原始 RGBA，二进制 IPC）。payload 里没有图，底图从这里来。 */
  frame: (label: string): Promise<ArrayBuffer> => getCaptureFrame(label),
  /**
   * 首帧已画好 / 或已经有错误可以显示——两种情况都该让窗口露出来。
   *
   * 顺手把**当下实测的**视口交给后端做 I4 自检。刻意在这里读
   * `window.innerWidth/innerHeight` 而不是从 React 状态里取：这一刻窗口刚布局完，
   * 是全链路里最新、也是唯一能反映"合成器最终摆成什么样"的数字。
   */
  ready: (label: string): Promise<void> =>
    markCaptureOverlayReady(label, window.innerWidth, window.innerHeight),
  cancel: (sessionId: string): Promise<void> => cancelCaptureOverlay(sessionId),
  /** 提交选区和 v2 操作层；后端用会话冻结帧生成权威 PNG。 */
  commit: (
    action: CaptureAction,
    selection: CaptureSelection,
    project: PinCanvasProject,
    origin: CaptureOrigin | null,
  ): Promise<CaptureActionResult> => commitCaptureAction(action, selection, project, origin),
  /** 选区翻译仍走后端裁剪：OCR 要的是原始像素，不是带标注的画布。 */
  translate: (selection: CaptureSelection): Promise<CaptureTranslationResult> =>
    translateCaptureSelection(selection),
  copyText: (text: string): Promise<void> => copyText(text),
};
