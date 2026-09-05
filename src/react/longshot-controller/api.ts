import {
  activateLongshotController,
  appendLongshotController,
  cancelLongshotController,
  markLongshotControllerReady,
  onCurrentWindowCloseRequested,
} from "../../js/api.ts";
import type {
  LongshotActivation,
  LongshotHandle,
  LongshotSnapshot,
} from "../../js/ipc-types.ts";

/** 控制页只通过共享 IPC 边界与原生窗口交互。 */
export const longshotControllerApi = {
  activate: (): Promise<LongshotActivation> => activateLongshotController(),
  append: (handle: LongshotHandle): Promise<LongshotSnapshot> => appendLongshotController(handle),
  ready: (): Promise<void> => markLongshotControllerReady(),
  cancel: (handle: LongshotHandle | null): Promise<void> => cancelLongshotController(handle),
  onCloseRequested: (callback: () => void): Promise<() => void> =>
    onCurrentWindowCloseRequested(callback),
};
