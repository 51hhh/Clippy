import { Copy, RotateCcw } from "lucide-react";
import { useSyncExternalStore } from "react";
import {
  enabledTranslationServices,
  translationProviderMeta,
} from "../../js/translation-providers";
import { t } from "../shared/i18n";
import {
  translationStore,
  type TranslationCard,
  type TranslationStore,
} from "./translationStore";

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

/** 认不出的语言码原样显示，总比显示成 "Detect automatically" 诚实。 */
function languageLabel(language: string): string {
  const key = LANGUAGE_KEYS[language];
  return key ? t(key) : language;
}

const ERROR_KEYS: Record<string, string> = {
  empty_input: "translation.error.emptyInput",
  input_too_large: "translation.error.inputTooLarge",
  sensitive_content: "translation.error.sensitive",
  missing_api_key: "translation.error.missingApiKey",
  incomplete_credentials: "translation.error.incompleteCredentials",
  keyring_unavailable: "translation.error.keyringUnavailable",
  clip_unavailable: "translation.error.clipUnavailable",
  image_unavailable: "translation.error.imageUnavailable",
  capture_unavailable: "translation.error.clipUnavailable",
  ocr_failed: "translation.error.ocrFailed",
  invalid_endpoint: "translation.error.configuration",
  unsupported_provider: "translation.error.configuration",
  no_service_enabled: "translation.error.noServiceEnabled",
  timeout: "translation.error.timeout",
  network: "translation.error.network",
  http_status: "translation.error.service",
  invalid_credentials: "translation.error.invalidCredentials",
  rate_limited: "translation.error.rateLimited",
  quota_exceeded: "translation.error.quotaExceeded",
  response_too_large: "translation.error.responseTooLarge",
  invalid_response: "translation.error.invalidResponse",
  provider_endpoint_broken: "translation.error.providerEndpointBroken",
  stale_request: "translation.error.stale",
  internal: "translation.error.generic",
};

function errorText(errorCode: string | null): string {
  return t(errorCode ? ERROR_KEYS[errorCode] || "translation.error.generic" : "translation.error.generic");
}

export function translationFeedbackText(feedback: string, errorCode: string | null): string {
  if (feedback === "complete") return t("translation.complete");
  if (feedback === "partial") return t("translation.partial");
  if (feedback === "error") return errorText(errorCode);
  return "";
}

/** 单张卡的状态行：进行中、失败原因、复制反馈或检测到的源语言 */
export function translationCardStatusText(card: TranslationCard): string {
  if (card.loading) return t("translation.working");
  if (card.errorCode) return errorText(card.errorCode);
  if (card.copyFeedback === "copied") return t("translation.copied");
  if (card.copyFeedback === "copy_failed") return t("translation.copyFailed");
  return card.detectedLanguage
    ? t("translation.detected", { language: card.detectedLanguage })
    : t("translation.result");
}

export function TranslationPanel({ store = translationStore }: { store?: TranslationStore }) {
  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  const { clip, config } = snapshot;
  if (!clip || !config) return null;

  // 未启用任何服务时不假装某个默认服务，直接告诉用户当前没有可用目标。
  const services = enabledTranslationServices(config.translation_services);
  const target = config.translation_target_language || "en";
  const targetLabel = languageLabel(target);
  const actionLabel = t(clip.content_type === "image"
    ? snapshot.loading ? "translation.ocrTranslating" : "translation.ocrAndTranslate"
    : snapshot.loading ? "translation.translating" : "translation.translate");
  const feedback = translationFeedbackText(snapshot.feedback, snapshot.errorCode);
  const busy = snapshot.loading || snapshot.cards.some((card) => card.loading);

  return (
    <section className="translation-panel" aria-labelledby="translation-title">
      <div className="translation-header">
        <div className="translation-heading">
          <h2 id="translation-title" className="translation-title">{t("translation.title")}</h2>
          <p className="translation-destination">
            <span>{t("translation.target", { language: targetLabel })}</span>
            <span aria-hidden="true">·</span>
            <span>{t("translation.destination")}:</span>
            {/* 启用的服务在下方逐行列出；一个都没有时就在这行说清楚。 */}
            {services.length === 0 && (
              <>
                <strong>{t("translation.providerNone")}</strong>
                <span aria-hidden="true">·</span>
                <span className="translation-endpoint">{t("translation.endpointUnavailable")}</span>
              </>
            )}
          </p>
          {services.length > 0 && (
            <ul className="translation-destinations">
              {services.map((service) => {
                const meta = translationProviderMeta(service.provider);
                return (
                  <li key={service.provider}>
                    <strong>{t(meta.nameKey)}</strong>
                    <span className="translation-endpoint">
                      {service.endpoint || meta.defaultEndpoint}
                    </span>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        <button
          id="translation-action-react"
          className="translation-action"
          type="button"
          disabled={clip.is_sensitive || busy}
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
          data-state={snapshot.feedback === "complete"
            ? "success"
            : snapshot.feedback === "partial" ? "warning" : "error"}
          role="status"
          aria-live="polite"
        >
          {feedback}
        </p>
      )}
      {snapshot.cards.map((card) => {
        const meta = translationProviderMeta(card.provider);
        const providerLabel = t(meta.nameKey);
        return (
          <div className="translation-result" key={card.provider} data-provider={card.provider}>
            <div className="translation-result-header">
              <span className="translation-result-provider">{providerLabel}</span>
              <span
                className="translation-detected"
                data-state={card.errorCode ? "error" : "info"}
                role="status"
                aria-live="polite"
              >
                {translationCardStatusText(card)}
              </span>
              {/* 源文本本来就是目标语言时后端会换向，实际目标必须如实标出来。 */}
              {card.targetLanguage && card.targetLanguage !== target && (
                <span className="translation-card-target">
                  {t("translation.target", { language: languageLabel(card.targetLanguage) })}
                </span>
              )}
              <span className="translation-card-actions">
                <button
                  className="translation-copy"
                  type="button"
                  disabled={!card.translatedText}
                  aria-label={`${t("translation.copy")} — ${providerLabel}`}
                  title={t("translation.copy")}
                  onClick={() => void store.copy(card.provider)}
                >
                  <Copy size={15} />
                </button>
                <button
                  className="translation-copy translation-retry"
                  type="button"
                  disabled={clip.is_sensitive || busy}
                  aria-label={`${t("translation.retry")} — ${providerLabel}`}
                  title={t("translation.retry")}
                  onClick={() => void store.retry(card.provider)}
                >
                  <RotateCcw size={15} />
                </button>
              </span>
            </div>
            {card.translatedText && (
              <pre className="translation-result-text" tabIndex={0}>{card.translatedText}</pre>
            )}
          </div>
        );
      })}
    </section>
  );
}
