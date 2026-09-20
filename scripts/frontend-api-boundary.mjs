/** 唯一允许直接访问 Tauri 的前端领域模块；公共调用方仍只导入 src/js/api.ts。 */
export const FRONTEND_API_MODULES = Object.freeze([
  "src/js/api/capture.ts",
  "src/js/api/clipboard.ts",
  "src/js/api/pin.ts",
  "src/js/api/recording.ts",
  "src/js/api/settings.ts",
  "src/js/api/viewer.ts",
]);
