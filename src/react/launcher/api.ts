import {
  actionLauncherReady,
  cancelAction,
  closeActionLauncher,
  discoverActions,
  getActionLauncherSettings,
  onCurrentWindowCloseRequested,
  prepareAction,
  runAction,
  startActionLauncherDrag,
} from "../../js/api.ts";

/** 生产与 DOM 验收共享一个服务合同；组件不直接接触 Tauri。 */
export const launcherApi = {
  discover: discoverActions,
  prepare: prepareAction,
  run: runAction,
  cancel: cancelAction,
  settings: getActionLauncherSettings,
  ready: actionLauncherReady,
  close: closeActionLauncher,
  startDrag: startActionLauncherDrag,
  onCloseRequested: onCurrentWindowCloseRequested,
};

export type LauncherServices = typeof launcherApi;
