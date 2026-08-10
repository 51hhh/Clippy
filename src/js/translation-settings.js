/**
 * translation-settings.js — 翻译服务配置与系统密钥状态
 */

import {
  deleteTranslationApiKey,
  hasTranslationApiKey,
  setTranslationApiKey,
} from "./api.js";
import * as i18n from "../i18n/i18n.js";

const PROVIDERS = {
  libretranslate: {
    nameKey: "settings.translation.providerLibre",
    endpoint: "https://libretranslate.com",
    model: "",
  },
  openai_compatible: {
    nameKey: "settings.translation.providerOpenAI",
    endpoint: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
  },
};

const STATUS_KEYS = {
  checking: "settings.translation.keyChecking",
  stored: "settings.translation.keyStored",
  missing: "settings.translation.keyMissing",
  saving: "settings.translation.keySaving",
  deleting: "settings.translation.keyDeleting",
  error: "settings.translation.keyError",
};

function getRequiredElement(root, id) {
  const element = root.getElementById(id);
  if (!element) throw new Error(`Missing translation settings element: #${id}`);
  return element;
}

function normalizeProvider(value) {
  return PROVIDERS[value] ? value : "libretranslate";
}

/**
 * 初始化翻译设置组件。调用方只负责表单配置的装载与收集。
 * @param {{root?: Document, showToast?: (message: string) => void}} options
 */
export function initTranslationSettings({ root = document, showToast = () => {} } = {}) {
  const providerSelect = getRequiredElement(root, "translation-provider-select");
  const endpointInput = getRequiredElement(root, "translation-endpoint-input");
  const modelInput = getRequiredElement(root, "translation-model-input");
  const sourceLanguageSelect = getRequiredElement(root, "translation-source-language-select");
  const targetLanguageSelect = getRequiredElement(root, "translation-target-language-select");
  const apiKeyInput = getRequiredElement(root, "translation-api-key-input");
  const keySaveBtn = getRequiredElement(root, "translation-key-save-btn");
  const keyDeleteBtn = getRequiredElement(root, "translation-key-delete-btn");
  const keyStatusDot = getRequiredElement(root, "translation-key-status-dot");
  const keyStatusText = getRequiredElement(root, "translation-key-status-text");
  const serviceName = getRequiredElement(root, "translation-service-name");

  let lastProvider = "libretranslate";
  let requestId = 0;
  let keyStatus = { phase: "checking", detail: "", hasKey: false };

  function currentProvider() {
    return normalizeProvider(providerSelect.value);
  }

  function updateKeyButtons() {
    const busy = ["checking", "saving", "deleting"].includes(keyStatus.phase);
    keySaveBtn.disabled = busy || !apiKeyInput.value.trim();
    keyDeleteBtn.disabled = busy;
  }

  function renderKeyStatus(phase, detail = "", hasKey = keyStatus.hasKey) {
    keyStatus = { phase, detail, hasKey };
    const tone = phase === "stored" ? "ready" : phase === "error" ? "unavailable" : "pending";
    keyStatusDot.className = `translation-status-dot ${tone}`;
    keyStatusText.textContent = i18n.t(STATUS_KEYS[phase] || STATUS_KEYS.error);
    keyStatusText.title = detail;
    keyDeleteBtn.hidden = !hasKey;
    updateKeyButtons();
  }

  function updateProvider(replaceDefaults) {
    const providerId = currentProvider();
    const provider = PROVIDERS[providerId];
    const previous = PROVIDERS[lastProvider];

    if (replaceDefaults) {
      const endpoint = endpointInput.value.trim();
      if (!endpoint || endpoint === previous?.endpoint) endpointInput.value = provider.endpoint;

      const model = modelInput.value.trim();
      if (!model || model === previous?.model) modelInput.value = provider.model;
      apiKeyInput.value = "";
    }

    serviceName.textContent = i18n.t(provider.nameKey);
    serviceName.title = endpointInput.value.trim();
    lastProvider = providerId;
    updateKeyButtons();
  }

  async function loadKeyStatus() {
    const provider = currentProvider();
    const activeRequest = ++requestId;
    renderKeyStatus("checking", "", false);
    try {
      const hasKey = await hasTranslationApiKey(provider);
      if (activeRequest !== requestId || provider !== currentProvider()) return;
      renderKeyStatus(hasKey ? "stored" : "missing", "", Boolean(hasKey));
    } catch (error) {
      if (activeRequest !== requestId || provider !== currentProvider()) return;
      renderKeyStatus("error", String(error), false);
    }
  }

  providerSelect.addEventListener("change", () => {
    requestId += 1;
    updateProvider(true);
    loadKeyStatus();
  });

  endpointInput.addEventListener("input", () => {
    serviceName.title = endpointInput.value.trim();
    endpointInput.removeAttribute("aria-invalid");
  });

  apiKeyInput.addEventListener("input", updateKeyButtons);
  apiKeyInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !keySaveBtn.disabled) {
      event.preventDefault();
      keySaveBtn.click();
    }
  });

  keySaveBtn.addEventListener("click", async () => {
    const provider = currentProvider();
    const apiKey = apiKeyInput.value.trim();
    if (!apiKey) return;

    const activeRequest = ++requestId;
    const previouslyStored = keyStatus.hasKey;
    apiKeyInput.value = "";
    renderKeyStatus("saving", "", previouslyStored);
    try {
      await setTranslationApiKey(provider, apiKey);
      if (activeRequest !== requestId || provider !== currentProvider()) return;
      renderKeyStatus("stored", "", true);
      showToast(i18n.t("settings.translation.keySaved"));
    } catch (error) {
      if (activeRequest !== requestId || provider !== currentProvider()) return;
      renderKeyStatus("error", String(error), previouslyStored);
      showToast(i18n.t("settings.translation.keySaveFailed"));
    }
  });

  keyDeleteBtn.addEventListener("click", async () => {
    const provider = currentProvider();
    const activeRequest = ++requestId;
    renderKeyStatus("deleting", "", true);
    try {
      await deleteTranslationApiKey(provider);
      if (activeRequest !== requestId || provider !== currentProvider()) return;
      apiKeyInput.value = "";
      renderKeyStatus("missing", "", false);
      showToast(i18n.t("settings.translation.keyDeleted"));
    } catch (error) {
      if (activeRequest !== requestId || provider !== currentProvider()) return;
      renderKeyStatus("error", String(error), true);
      showToast(i18n.t("settings.translation.keyDeleteFailed"));
    }
  });

  return {
    fill(config) {
      const providerId = normalizeProvider(config.translation_provider);
      providerSelect.value = providerId;
      lastProvider = providerId;
      const provider = PROVIDERS[providerId];
      endpointInput.value = config.translation_endpoint || provider.endpoint;
      modelInput.value = config.translation_model ?? provider.model;
      sourceLanguageSelect.value = config.translation_source_language || "auto";
      targetLanguageSelect.value = config.translation_target_language || "en";
      if (!sourceLanguageSelect.value) sourceLanguageSelect.value = "auto";
      if (!targetLanguageSelect.value) targetLanguageSelect.value = "en";
      updateProvider(false);
    },

    getConfig() {
      const endpoint = endpointInput.value.trim();
      let parsedEndpoint;
      try {
        if (!endpoint || /\s/.test(endpoint) || endpoint.includes("@")) throw new Error("invalid");
        parsedEndpoint = new URL(endpoint);
      } catch (_) {
        endpointInput.setAttribute("aria-invalid", "true");
        endpointInput.focus();
        throw new Error(i18n.t("settings.translation.endpointInvalid"));
      }
      const localHttp = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .some((prefix) => {
          if (endpoint === prefix) return true;
          const suffix = endpoint.startsWith(prefix) ? endpoint.slice(prefix.length) : "";
          return suffix.startsWith(":") || suffix.startsWith("/");
        });
      const isAllowed = (endpoint.startsWith("https://") && parsedEndpoint.protocol === "https:")
        || (localHttp && parsedEndpoint.protocol === "http:");
      if (!isAllowed) {
        endpointInput.setAttribute("aria-invalid", "true");
        endpointInput.focus();
        throw new Error(i18n.t("settings.translation.endpointInvalid"));
      }
      endpointInput.removeAttribute("aria-invalid");
      return {
        translation_provider: currentProvider(),
        translation_endpoint: endpoint,
        translation_model: modelInput.value.trim(),
        translation_source_language: sourceLanguageSelect.value,
        translation_target_language: targetLanguageSelect.value,
      };
    },

    loadKeyStatus,

    refreshLabels() {
      updateProvider(false);
      renderKeyStatus(keyStatus.phase, keyStatus.detail, keyStatus.hasKey);
    },
  };
}
