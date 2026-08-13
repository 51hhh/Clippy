import type { Rect, TranslationProvider } from "./types";
import { t } from "../shared/i18n";

const ERROR_KEYS: Record<string, string> = {
  empty_input: "capture.translation.error.emptyInput",
  input_too_large: "capture.translation.error.inputTooLarge",
  missing_api_key: "capture.translation.error.missingApiKey",
  keyring_unavailable: "capture.translation.error.keyringUnavailable",
  capture_unavailable: "capture.translation.error.captureUnavailable",
  ocr_failed: "capture.translation.error.ocrFailed",
  invalid_endpoint: "capture.translation.error.invalidEndpoint",
  unsupported_provider: "capture.translation.error.unsupportedProvider",
  timeout: "capture.translation.error.timeout",
  network: "capture.translation.error.network",
  http_status: "capture.translation.error.service",
  response_too_large: "capture.translation.error.responseTooLarge",
  invalid_response: "capture.translation.error.invalidResponse",
  stale_request: "capture.translation.error.stale",
  internal: "capture.translation.error.internal",
};

export function translationErrorMessage(reason: unknown): string {
  const match = /^translation\.([a-z_]+):/.exec(String(reason));
  return t(match && ERROR_KEYS[match[1]] ? ERROR_KEYS[match[1]] : "capture.translation.error.generic");
}

export function providerLabel(provider: TranslationProvider): string {
  return provider === "libretranslate" ? "LibreTranslate" : t("capture.translation.providerOpenAI");
}

export function isCurrentTranslation(currentGeneration: number, requestGeneration: number): boolean {
  return currentGeneration === requestGeneration;
}

export function translationPanelPosition(
  selection: Rect,
  viewportWidth: number,
  viewportHeight: number,
): { left: number; top: number } {
  const margin = 8;
  const width = Math.min(360, Math.max(1, viewportWidth - margin * 2));
  const estimatedHeight = Math.min(320, Math.max(1, viewportHeight - margin * 2));
  const left = Math.max(
    margin,
    Math.min(selection.x + selection.width - width, viewportWidth - width - margin),
  );
  const below = selection.y + selection.height + 58;
  const top = below + estimatedHeight <= viewportHeight - margin
    ? below
    : Math.max(margin, Math.min(selection.y - estimatedHeight - 8, viewportHeight - estimatedHeight - margin));
  return { left, top };
}
