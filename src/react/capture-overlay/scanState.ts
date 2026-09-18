import { t } from "../shared/i18n";

const ERROR_KEYS: Record<string, string> = {
  busy: "capture.scan.error.busy",
  image_too_large: "capture.scan.error.tooLarge",
  capture_unavailable: "capture.scan.error.unavailable",
};

/** IPC 只暴露稳定 code；未知对象和后端细节统一折叠成通用文案。 */
export function captureScanErrorMessage(reason: unknown): string {
  const code = reason && typeof reason === "object" && !Array.isArray(reason)
    && typeof (reason as { code?: unknown }).code === "string"
    ? (reason as { code: string }).code
    : null;
  return t(code && ERROR_KEYS[code] ? ERROR_KEYS[code] : "capture.scan.error.generic");
}

export function isCurrentCaptureScan(
  currentGeneration: number,
  generation: number,
  currentSelectionKey: string,
  selectionKey: string,
): boolean {
  return currentGeneration === generation && currentSelectionKey === selectionKey;
}
