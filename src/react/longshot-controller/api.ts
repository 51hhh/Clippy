import {
  activateLongshotController,
  appendLongshotController,
  cancelLongshotController,
  finishLongshotController,
  markLongshotControllerReady,
  onCurrentWindowCloseRequested,
} from "../../js/api.ts";
import type {
  LongshotActivation,
  LongshotHandle,
  LongshotOutputAction,
  LongshotOutputResult,
  LongshotSnapshot,
} from "../../js/ipc-types.ts";

/** 控制页只通过共享 IPC 边界与原生窗口交互。 */
export const longshotControllerApi = {
  activate: (): Promise<LongshotActivation> => activateLongshotController(),
  append: (handle: LongshotHandle): Promise<LongshotSnapshot> => appendLongshotController(handle),
  finish: (
    handle: LongshotHandle,
    action: LongshotOutputAction,
  ): Promise<LongshotOutputResult> => finishLongshotController(handle, action),
  ready: (): Promise<void> => markLongshotControllerReady(),
  cancel: (handle: LongshotHandle | null): Promise<void> => cancelLongshotController(handle),
  onCloseRequested: (callback: () => void): Promise<() => void> =>
    onCurrentWindowCloseRequested(callback),
};
