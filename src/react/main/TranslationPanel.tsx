import { Copy } from "lucide-react";
import { useSyncExternalStore } from "react";
import { t } from "../shared/i18n";
import { translationStore, type TranslationStore } from "./translationStore";

const PROVIDER_KEYS: Record<string, string> = {
  libretranslate: "settings.translation.providerLibre",
  openai_compatible: "settings.translation.providerOpenAI",
};

const LANGUAGE_KEYS: Record<string, string> = {
  en: "settings.translation.languageEnglish",
  zh: "settings.translation.languageChinese",
  "zh-CN": "settings.translation.languageChinese",
  ja: "settings.translation.languageJapanese",
  ko: "settings.translation.languageKorean",
  es: "settings.translation.languageSpanish",
  fr: "settings.translation.languageFrench",
  de: "settings.translation.languageGerman",
};

const ERROR_KEYS: Record<string, string> = {
  empty_input: "translation.error.emptyInput",
  input_too_large: "translation.error.inputTooLarge",
  sensitive_content: "translation.error.sensitive",
  missing_api_key: "translation.error.missingApiKey",
  keyring_unavailable: "translation.error.keyringUnavailable",
  clip_unavailable: "translation.error.clipUnavailable",
  image_unavailable: "translation.error.imageUnavailable",
  ocr_failed: "translation.error.ocrFailed",
  invalid_endpoint: "translation.error.configuration",
  unsupported_provider: "translation.error.configuration",
  timeout: "translation.error.timeout",
  network: "translation.error.network",
  http_status: "translation.error.service",
  response_too_large: "translation.error.responseTooLarge",
  invalid_response: "translation.error.invalidResponse",
  stale_request: "translation.error.stale",
  internal: "translation.error.generic",
};

export function translationFeedbackText(feedback: string, errorCode: string | null): string {
  if (feedback === "complete") return t("translation.complete");
  if (feedback === "copied") return t("translation.copied");
  if (feedback === "copy_failed") return t("translation.copyFailed");
  if (feedback === "error") return t(errorCode ? ERROR_KEYS[errorCode] || "translation.error.generic" : "translation.error.generic");
  return "";
}

export function TranslationPanel({ store = translationStore }: { store?: TranslationStore }) {
  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  const { clip, config } = snapshot;
  if (!clip || !config) return null;

  const provider = config.translation_provider || "libretranslate";
  const target = config.translation_target_language || "en";
  const providerLabel = t(PROVIDER_KEYS[provider] || "translation.providerUnknown");
  const targetLabel = t(LANGUAGE_KEYS[target] || "settings.translation.languageAuto");
  const actionLabel = t(clip.content_type === "image"
    ? snapshot.loading ? "translation.ocrTranslating" : "translation.ocrAndTranslate"
    : snapshot.loading ? "translation.translating" : "translation.translate");
  const feedback = translationFeedbackText(snapshot.feedback, snapshot.errorCode);

  return (
    <section className="translation-panel" aria-labelledby="translation-title">
      <div className="translation-header">
        <div className="translation-heading">
          <h2 id="translation-title" className="translation-title">{t("translation.title")}</h2>
          <p className="translation-destination">
            <span>{t("translation.destination")}:</span>
            <strong>{providerLabel}</strong>
            <span aria-hidden="true">·</span>
            <span>{t("translation.target", { language: targetLabel })}</span>
            <span aria-hidden="true">·</span>
            <span className="translation-endpoint">
              {config.translation_endpoint || t("translation.endpointUnavailable")}
            </span>
          </p>
        </div>
        <button
          id="translation-action-react"
          className="translation-action"
          type="button"
          disabled={clip.is_sensitive || snapshot.loading}
          aria-describedby={clip.is_sensitive
            ? "translation-privacy-react translation-sensitive-react"
            : "translation-privacy-react"}
          onClick={() => void store.translate()}
        >
          {actionLabel}
        </button>
      </div>
      <p id="translation-privacy-react" className="translation-privacy">{t("translation.privacy")}</p>
      {clip.is_sensitive && (
        <p id="translation-sensitive-react" className="translation-sensitive" role="status" tabIndex={-1}>
          {t("translation.sensitive")}
        </p>
      )}
      {snapshot.loading && (
        <p className="translation-status" role="status" aria-live="polite">
          {t("translation.working")}
        </p>
      )}
      {feedback && (
        <p
          className="translation-status"
          data-state={snapshot.feedback === "error" || snapshot.feedback === "copy_failed" ? "error" : "success"}
          role="status"
          aria-live="polite"
        >
          {feedback}
        </p>
      )}
      {snapshot.translatedText && (
        <div className="translation-result">
          <div className="translation-result-header">
            <span className="translation-detected">
              {snapshot.detectedLanguage
                ? t("translation.detected", { language: snapshot.detectedLanguage })
                : t("translation.result")}
            </span>
            <button
              className="translation-copy"
              type="button"
              aria-label={t("translation.copy")}
              title={t("translation.copy")}
              onClick={() => void store.copy()}
            >
              <Copy size={15} />
            </button>
          </div>
          <pre className="translation-result-text" tabIndex={0}>{snapshot.translatedText}</pre>
        </div>
      )}
    </section>
  );
}
