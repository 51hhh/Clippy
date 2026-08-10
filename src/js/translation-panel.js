/**
 * translation-panel.js — 主预览中的显式翻译工作区
 *
 * 聚焦变化只更新本地 UI。文本外发和图片 OCR 仅由用户点击触发。
 */

import { translateClip } from "./api.js";
import { t } from "../i18n/i18n.js";

const PROVIDER_KEYS = {
  libretranslate: "settings.translation.providerLibre",
  openai_compatible: "settings.translation.providerOpenAI",
};

const LANGUAGE_KEYS = {
  en: "settings.translation.languageEnglish",
  zh: "settings.translation.languageChinese",
  "zh-CN": "settings.translation.languageChinese",
  ja: "settings.translation.languageJapanese",
  ko: "settings.translation.languageKorean",
  es: "settings.translation.languageSpanish",
  fr: "settings.translation.languageFrench",
  de: "settings.translation.languageGerman",
};

const ERROR_KEYS = {
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

let elements = {};
let currentClip = null;
let currentConfig = {};
let generation = 0;
let translatedText = "";
let loading = false;

export function init(config = {}) {
  elements = {
    panel: document.getElementById("translation-panel"),
    provider: document.getElementById("translation-provider"),
    target: document.getElementById("translation-target"),
    endpoint: document.getElementById("translation-endpoint"),
    action: document.getElementById("translation-action"),
    privacy: document.getElementById("translation-privacy"),
    sensitive: document.getElementById("translation-sensitive"),
    status: document.getElementById("translation-status"),
    result: document.getElementById("translation-result"),
    detected: document.getElementById("translation-detected"),
    resultText: document.getElementById("translation-result-text"),
    copy: document.getElementById("translation-copy"),
  };

  if (!elements.panel || !elements.action || !elements.copy) return;
  elements.action.addEventListener("click", requestTranslation);
  elements.copy.addEventListener("click", copyResult);
  currentConfig = { ...config };
  renderStaticLabels();
  renderDestination();
  updateClip(null);
}

/** 更新当前条目，并使此前所有异步结果失效。 */
export function updateClip(clip) {
  generation += 1;
  currentClip = clip || null;
  loading = false;
  translatedText = "";
  resetFeedback();

  if (!elements.panel) return;
  elements.panel.hidden = !currentClip;
  if (!currentClip) return;

  const sensitive = currentClip.is_sensitive === true;
  elements.sensitive.hidden = !sensitive;
  elements.action.disabled = sensitive;
  renderActionLabel();
}

/** 配置变化时刷新目的地，并丢弃使用旧服务发起的在途结果。 */
export function updateConfig(config = {}) {
  generation += 1;
  currentConfig = { ...currentConfig, ...config };
  loading = false;
  translatedText = "";
  resetFeedback();
  renderStaticLabels();
  renderDestination();

  if (currentClip) {
    const sensitive = currentClip.is_sensitive === true;
    elements.sensitive.hidden = !sensitive;
    elements.action.disabled = sensitive;
    renderActionLabel();
  }
}

/** 释放当前翻译结果中的用户内容。 */
export function clear() {
  updateClip(null);
}

/** 预览由键盘打开时，将焦点送入可操作区或敏感保护说明。 */
export function focusAction() {
  if (!currentClip || elements.panel?.hidden) return false;
  const target = currentClip.is_sensitive === true ? elements.sensitive : elements.action;
  target?.focus();
  return document.activeElement === target;
}

async function requestTranslation() {
  if (!currentClip || currentClip.is_sensitive === true || loading) return;

  const clipId = currentClip.id;
  const requestGeneration = ++generation;
  loading = true;
  translatedText = "";
  resetFeedback();
  elements.action.disabled = true;
  elements.action.textContent = t(
    currentClip.content_type === "image"
      ? "translation.ocrTranslating"
      : "translation.translating",
  );
  setStatus("loading", "translation.working");

  try {
    const result = await translateClip(clipId);
    if (!isCurrent(requestGeneration, clipId)) return;
    if (!result || typeof result.translated_text !== "string" || !result.translated_text.trim()) {
      throw new Error("translation.invalid_response:");
    }

    translatedText = result.translated_text;
    elements.resultText.textContent = translatedText;
    elements.detected.textContent = result.detected_source_language
      ? t("translation.detected", { language: result.detected_source_language })
      : t("translation.result");
    elements.result.hidden = false;
    setStatus("success", "translation.complete");
  } catch (error) {
    if (!isCurrent(requestGeneration, clipId)) return;
    setStatus("error", errorTranslationKey(error));
  } finally {
    if (isCurrent(requestGeneration, clipId)) {
      loading = false;
      elements.action.disabled = false;
      renderActionLabel();
    }
  }
}

async function copyResult() {
  if (!translatedText) return;
  try {
    await navigator.clipboard.writeText(translatedText);
    setStatus("success", "translation.copied");
  } catch {
    setStatus("error", "translation.copyFailed");
  }
}

function isCurrent(requestGeneration, clipId) {
  return generation === requestGeneration && currentClip?.id === clipId;
}

function renderActionLabel() {
  if (!elements.action || !currentClip || loading) return;
  elements.action.textContent = t(
    currentClip.content_type === "image"
      ? "translation.ocrAndTranslate"
      : "translation.translate",
  );
}

function renderStaticLabels() {
  if (!elements.copy) return;
  const copyLabel = t("translation.copy");
  elements.copy.setAttribute("aria-label", copyLabel);
  elements.copy.title = copyLabel;
}

function renderDestination() {
  if (!elements.provider) return;
  const provider = currentConfig.translation_provider || "libretranslate";
  const target = currentConfig.translation_target_language || "en";
  const endpoint = currentConfig.translation_endpoint || t("translation.endpointUnavailable");

  elements.provider.textContent = t(PROVIDER_KEYS[provider] || "translation.providerUnknown");
  elements.target.textContent = t("translation.target", { language: languageLabel(target) });
  elements.endpoint.textContent = endpoint;
}

function languageLabel(language) {
  const key = LANGUAGE_KEYS[language];
  return key ? t(key) : language;
}

function resetFeedback() {
  if (!elements.result) return;
  elements.result.hidden = true;
  elements.resultText.textContent = "";
  elements.detected.textContent = "";
  elements.status.hidden = true;
  elements.status.textContent = "";
  delete elements.status.dataset.state;
  if (elements.sensitive) elements.sensitive.hidden = true;
}

function setStatus(state, key) {
  elements.status.dataset.state = state;
  elements.status.textContent = t(key);
  elements.status.hidden = false;
}

function errorTranslationKey(error) {
  const message = typeof error === "string" ? error : String(error?.message || error || "");
  const match = message.match(/(?:^|\s)translation\.([a-z_]+)(?=:|$)/);
  return match ? (ERROR_KEYS[match[1]] || "translation.error.generic") : "translation.error.generic";
}

export const __test__ = { errorTranslationKey, languageLabel };
