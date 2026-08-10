import type { Rect, TranslationProvider } from "./types";

const ERROR_MESSAGES: Record<string, string> = {
  empty_input: "No text was found in this selection.",
  input_too_large: "The recognized text is too large to translate.",
  missing_api_key: "Add an API key in Translation settings.",
  keyring_unavailable: "Secure credential storage is unavailable.",
  capture_unavailable: "This capture selection is no longer available.",
  ocr_failed: "Local OCR could not extract text from this selection.",
  invalid_endpoint: "The translation endpoint is invalid.",
  unsupported_provider: "The configured translation provider is unsupported.",
  timeout: "The translation request timed out.",
  network: "The translation service could not be reached.",
  http_status: "The translation service rejected the request.",
  response_too_large: "The translation response is too large.",
  invalid_response: "The translation service returned an invalid response.",
  stale_request: "A newer translation request replaced this one.",
  internal: "Translation is temporarily unavailable.",
};

export function translationErrorMessage(reason: unknown): string {
  const match = /^translation\.([a-z_]+):/.exec(String(reason));
  return match ? (ERROR_MESSAGES[match[1]] || "Translation failed.") : "Translation failed.";
}

export function providerLabel(provider: TranslationProvider): string {
  return provider === "libretranslate" ? "LibreTranslate" : "OpenAI compatible";
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
