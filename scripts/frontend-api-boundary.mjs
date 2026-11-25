/** 唯一允许直接访问 Tauri 的 legacy 前端领域模块；公共调用方仍只导入 api.ts。 */
export const FRONTEND_API_MODULES = Object.freeze([
  "apps/legacy-tauri/frontend/js/api/actions.ts",
  "apps/legacy-tauri/frontend/js/api/capture.ts",
  "apps/legacy-tauri/frontend/js/api/clipboard.ts",
  "apps/legacy-tauri/frontend/js/api/pin.ts",
  "apps/legacy-tauri/frontend/js/api/recording.ts",
  "apps/legacy-tauri/frontend/js/api/settings.ts",
  "apps/legacy-tauri/frontend/js/api/viewer.ts",
]);
