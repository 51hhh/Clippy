/**
 * translation-settings.js — 翻译服务配置与系统密钥状态
 *
 * 配置以 `translation_services` 列表形式存储：每个服务各自保留 endpoint/model/region/project，
 * 未启用的服务也保留自己的配置，用户来回切换不会丢。provider 选择器只决定“正在编辑哪个服务”，
 * 启用与否由各自的开关控制，可以同时启用多个服务并行翻译。
 */

import {
  deleteTranslationApiKey,
  hasTranslationApiKey,
  setTranslationApiKey,
} from "./api.ts";
import {
  DEFAULT_TRANSLATION_PROVIDER,
  TRANSLATION_PROVIDER_IDS,
  enabledTranslationServices,
  normalizeTranslationProvider,
  primaryTranslationService,
  translationProviderMeta,
} from "./translation-providers.ts";
import * as i18n from "../i18n/i18n.js";

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

/** 每个服务一份配置，缺失的服务补成未启用的空配置 */
function emptyServices() {
  return TRANSLATION_PROVIDER_IDS.map((provider) => ({
    provider,
    enabled: false,
    endpoint: "",
    model: "",
    region: "",
    project: "",
  }));
}

/**
 * 校验端点：空值表示沿用内置默认值，非空则只允许 HTTPS 或本机 HTTP。
 * 与后端 `validate_endpoint` 保持同一套规则，避免前端放过后端会拒绝的地址。
 */
function isAllowedEndpoint(endpoint) {
  if (!endpoint) return true;
  if (/\s/.test(endpoint) || endpoint.includes("@")) return false;
  let parsed;
  try {
    parsed = new URL(endpoint);
  } catch (_) {
    return false;
  }
  if (endpoint.startsWith("https://") && parsed.protocol === "https:") return true;
  const localHttp = ["http://localhost", "http://127.0.0.1", "http://[::1]"].some((prefix) => {
    if (endpoint === prefix) return true;
    const suffix = endpoint.startsWith(prefix) ? endpoint.slice(prefix.length) : "";
    return suffix.startsWith(":") || suffix.startsWith("/");
  });
  return localHttp && parsed.protocol === "http:";
}

/**
 * 初始化翻译设置组件。调用方只负责表单配置的装载与收集。
 * @param {{root?: Document, showToast?: (message: string) => void}} options
 */
export function initTranslationSettings({ root = document, showToast = () => {} } = {}) {
  const providerSelect = getRequiredElement(root, "translation-provider-select");
  const enabledToggle = getRequiredElement(root, "translation-service-enabled");
  const endpointInput = getRequiredElement(root, "translation-endpoint-input");
  const modelField = getRequiredElement(root, "translation-model-field");
  const modelInput = getRequiredElement(root, "translation-model-input");
  const regionField = getRequiredElement(root, "translation-region-field");
  const regionInput = getRequiredElement(root, "translation-region-input");
  const projectField = getRequiredElement(root, "translation-project-field");
  const projectInput = getRequiredElement(root, "translation-project-input");
  const sourceLanguageSelect = getRequiredElement(root, "translation-source-language-select");
  const targetLanguageSelect = getRequiredElement(root, "translation-target-language-select");
  const preferredList = getRequiredElement(root, "translation-preferred-languages");
  const preferredInputs = Array.from(
    preferredList.querySelectorAll('input[type="checkbox"]'),
  );
  const apiKeyInput = getRequiredElement(root, "translation-api-key-input");
  const apiSecretInput = getRequiredElement(root, "translation-api-secret-input");
  const fallbackHint = getRequiredElement(root, "translation-fallback-hint");
  const keySaveBtn = getRequiredElement(root, "translation-key-save-btn");
  const keyDeleteBtn = getRequiredElement(root, "translation-key-delete-btn");
  const keyStatusDot = getRequiredElement(root, "translation-key-status-dot");
  const keyStatusText = getRequiredElement(root, "translation-key-status-text");
  const serviceName = getRequiredElement(root, "translation-service-name");

  let lastProvider = DEFAULT_TRANSLATION_PROVIDER;
  let services = emptyServices();
  let requestId = 0;
  let keyStatus = { phase: "checking", detail: "", hasKey: false };
  /** 备选语言的优先级顺序。勾选框按界面顺序排列，但已保存的顺序不能被重排覆盖。 */
  let preferredOrder = [];

  function currentProvider() {
    return normalizeTranslationProvider(providerSelect.value);
  }

  function serviceEntry(providerId) {
    return services.find((service) => service.provider === providerId);
  }

  /** 表单当前内容写回内存中的服务配置，切换服务前必须调用 */
  function captureForm(providerId) {
    const service = serviceEntry(providerId);
    if (!service) return;
    service.endpoint = endpointInput.value.trim();
    service.model = modelInput.value.trim();
    service.region = regionInput.value.trim();
    service.project = projectInput.value.trim();
  }

  /** 端点与模型留空即沿用内置默认值，因此默认值只作为 placeholder 展示 */
  function applyForm(providerId) {
    const service = serviceEntry(providerId);
    const meta = translationProviderMeta(providerId);
    enabledToggle.checked = Boolean(service?.enabled);
    endpointInput.value = service?.endpoint ?? "";
    endpointInput.placeholder = meta.defaultEndpoint;
    modelInput.value = service?.model ?? "";
    modelInput.placeholder = meta.defaultModel;
    regionInput.value = service?.region ?? "";
    projectInput.value = service?.project ?? "";
    endpointInput.removeAttribute("aria-invalid");
  }

  /** 只显示当前服务真正需要的字段，避免让用户填对该服务无意义的参数 */
  function applyFieldVisibility(providerId) {
    const meta = translationProviderMeta(providerId);
    modelField.hidden = !meta.needsModel;
    regionField.hidden = !meta.needsRegion;
    projectField.hidden = !meta.needsProject;
    apiSecretInput.hidden = !meta.needsSecret;
    fallbackHint.hidden = !meta.hasWebFallback;
  }

  function updateKeyButtons() {
    const busy = ["checking", "saving", "deleting"].includes(keyStatus.phase);
    const meta = translationProviderMeta(currentProvider());
    // 双字段服务少填一半凭据会在翻译时才报错，这里直接拦住保存。
    const secretMissing = Boolean(meta.needsSecret) && !apiSecretInput.value.trim();
    keySaveBtn.disabled = busy || !apiKeyInput.value.trim() || secretMissing;
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

  /**
   * 目的地摘要列出所有启用的服务：多服务并行时，用户需要一眼看到文本会发往哪几个端点。
   * 正在编辑的服务用输入框里的实时值，其余用已保存的值。
   */
  function renderDestination() {
    const providerId = currentProvider();
    const enabled = enabledTranslationServices(services);
    if (!enabled.length) {
      serviceName.textContent = i18n.t("settings.translation.destinationNone");
      serviceName.title = "";
      return;
    }
    serviceName.textContent = enabled
      .map((service) => i18n.t(translationProviderMeta(service.provider).nameKey))
      .join(", ");
    serviceName.title = enabled
      .map((service) => {
        const endpoint = service.provider === providerId
          ? endpointInput.value.trim()
          : service.endpoint;
        return endpoint || translationProviderMeta(service.provider).defaultEndpoint;
      })
      .join("\n");
  }

  function updateProvider() {
    const providerId = currentProvider();
    applyFieldVisibility(providerId);
    renderDestination();
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
    captureForm(lastProvider);
    const providerId = currentProvider();
    applyForm(providerId);
    apiKeyInput.value = "";
    apiSecretInput.value = "";
    updateProvider();
    loadKeyStatus();
  });

  /** 新勾选的语言追加到末尾：先勾的优先级更高，取消勾选只把它移出列表 */
  for (const input of preferredInputs) {
    input.addEventListener("change", () => {
      preferredOrder = input.checked
        ? [...preferredOrder.filter((language) => language !== input.value), input.value]
        : preferredOrder.filter((language) => language !== input.value);
    });
  }

  enabledToggle.addEventListener("change", () => {
    const service = serviceEntry(currentProvider());
    if (service) service.enabled = enabledToggle.checked;
    renderDestination();
  });

  endpointInput.addEventListener("input", () => {
    renderDestination();
    endpointInput.removeAttribute("aria-invalid");
  });

  apiKeyInput.addEventListener("input", updateKeyButtons);
  apiSecretInput.addEventListener("input", updateKeyButtons);
  for (const input of [apiKeyInput, apiSecretInput]) {
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !keySaveBtn.disabled) {
        event.preventDefault();
        keySaveBtn.click();
      }
    });
  }

  keySaveBtn.addEventListener("click", async () => {
    const provider = currentProvider();
    const apiKey = apiKeyInput.value.trim();
    const apiSecret = apiSecretInput.value.trim();
    if (!apiKey) return;
    if (translationProviderMeta(provider).needsSecret && !apiSecret) return;

    const activeRequest = ++requestId;
    const previouslyStored = keyStatus.hasKey;
    apiKeyInput.value = "";
    apiSecretInput.value = "";
    renderKeyStatus("saving", "", previouslyStored);
    try {
      await setTranslationApiKey(provider, apiKey, apiSecret || undefined);
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
      apiSecretInput.value = "";
      renderKeyStatus("missing", "", false);
      showToast(i18n.t("settings.translation.keyDeleted"));
    } catch (error) {
      if (activeRequest !== requestId || provider !== currentProvider()) return;
      renderKeyStatus("error", String(error), true);
      showToast(i18n.t("settings.translation.keyDeleteFailed"));
    }
  });

  /** 端点非法时把选择器切回出问题的服务再聚焦，否则用户看不到是哪个服务错了 */
  function rejectEndpoint(providerId) {
    if (providerId !== currentProvider()) {
      providerSelect.value = providerId;
      applyForm(providerId);
      updateProvider();
    }
    endpointInput.setAttribute("aria-invalid", "true");
    endpointInput.focus();
    throw new Error(i18n.t("settings.translation.endpointInvalid"));
  }

  return {
    fill(config) {
      services = emptyServices();
      for (const stored of config.translation_services ?? []) {
        // 认不出的 provider 名直接丢弃，不要污染已知服务的配置。
        const service = serviceEntry(stored?.provider);
        if (!service) continue;
        service.enabled = Boolean(stored.enabled);
        service.endpoint = stored.endpoint ?? "";
        service.model = stored.model ?? "";
        service.region = stored.region ?? "";
        service.project = stored.project ?? "";
      }

      const providerId = normalizeTranslationProvider(
        primaryTranslationService(services)?.provider,
      );
      providerSelect.value = providerId;
      applyForm(providerId);
      sourceLanguageSelect.value = config.translation_source_language || "auto";
      targetLanguageSelect.value = config.translation_target_language || "en";
      if (!sourceLanguageSelect.value) sourceLanguageSelect.value = "auto";
      if (!targetLanguageSelect.value) targetLanguageSelect.value = "en";

      // 界面上没有的语言码丢弃，否则保存时会静默重排成勾选框的顺序。
      const available = preferredInputs.map((input) => input.value);
      preferredOrder = (config.preferred_languages ?? [])
        .filter((language) => available.includes(language))
        .filter((language, index, list) => list.indexOf(language) === index);
      for (const input of preferredInputs) {
        input.checked = preferredOrder.includes(input.value);
      }
      updateProvider();
    },

    getConfig() {
      const providerId = currentProvider();
      captureForm(providerId);
      // 未显示的服务也要校验：它的端点同样会被后端使用。
      for (const service of services) {
        if (!isAllowedEndpoint(service.endpoint)) rejectEndpoint(service.provider);
      }
      endpointInput.removeAttribute("aria-invalid");

      return {
        // 启用状态由各服务自己的开关决定，选择器只表示正在编辑哪一个。
        translation_services: services.map((service) => ({ ...service })),
        translation_source_language: sourceLanguageSelect.value,
        translation_target_language: targetLanguageSelect.value,
        // 全不勾即空数组，后端据此沿用「目标 + 源」这一对语言。
        preferred_languages: [...preferredOrder],
      };
    },

    loadKeyStatus,

    refreshLabels() {
      updateProvider();
      renderKeyStatus(keyStatus.phase, keyStatus.detail, keyStatus.hasKey);
    },
  };
}
