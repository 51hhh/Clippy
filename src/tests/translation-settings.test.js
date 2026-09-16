import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  clearTranslationHistory: vi.fn(),
  deleteTranslationApiKey: vi.fn(),
  hasTranslationApiKey: vi.fn(),
  setTranslationApiKey: vi.fn(),
}));

import {
  clearTranslationHistory,
  deleteTranslationApiKey,
  hasTranslationApiKey,
  setTranslationApiKey,
} from "../js/api.ts";
import { initTranslationSettings } from "../js/translation-settings.js";
import * as i18n from "../i18n/i18n.js";

const PROVIDER_IDS = [
  "libretranslate",
  "openai_compatible",
  "deepl",
  "google",
  "bing",
  "youdao",
];

/** 备选语言勾选框，顺序与 settings.html 一致 */
const PREFERRED_LANGUAGES = ["en", "zh", "ja", "ko", "es", "fr", "de"]
  .map((code) => `<label><input type="checkbox" value="${code}">${code}</label>`)
  .join("");

function setupDom() {
  const options = PROVIDER_IDS.map((id) => `<option value="${id}">${id}</option>`).join("");
  document.body.innerHTML = `
    <select id="translation-provider-select">${options}</select>
    <input id="translation-service-enabled" type="checkbox">
    <input id="translation-endpoint-input">
    <div id="translation-model-field"><input id="translation-model-input"></div>
    <div id="translation-region-field" hidden><input id="translation-region-input"></div>
    <div id="translation-project-field" hidden><input id="translation-project-input"></div>
    <select id="translation-source-language-select">
      <option value="auto">Auto</option><option value="en">English</option>
    </select>
    <select id="translation-target-language-select">
      <option value="en">English</option><option value="zh">Chinese</option>
    </select>
    <div id="translation-preferred-languages">${PREFERRED_LANGUAGES}</div>
    <input id="translation-api-key-input" type="password">
    <input id="translation-api-secret-input" type="password" hidden>
    <div id="translation-fallback-hint" hidden></div>
    <button id="translation-key-save-btn"></button>
    <button id="translation-key-delete-btn" hidden></button>
    <span id="translation-key-status-dot"></span>
    <span id="translation-key-status-text"></span>
    <strong id="translation-service-name"></strong>
    <button id="translation-history-clear-btn"></button>
    <span id="translation-history-status-text"></span>
  `;
}

/** 只启用一个服务的配置，其余服务保持未启用但保留自己的字段 */
function configWith(provider, overrides = {}) {
  return {
    translation_services: PROVIDER_IDS.map((id) => ({
      provider: id,
      enabled: id === provider,
      endpoint: "",
      model: "",
      region: "",
      project: "",
      ...(id === provider ? overrides : {}),
    })),
    translation_source_language: "auto",
    translation_target_language: "en",
  };
}

function serviceOf(config, provider) {
  return config.translation_services.find((service) => service.provider === provider);
}

function element(id) {
  return document.getElementById(id);
}

function preferred(code) {
  return document.querySelector(`#translation-preferred-languages input[value="${code}"]`);
}

describe("translation settings", () => {
  let showToast;
  let settings;

  beforeEach(() => {
    vi.clearAllMocks();
    setupDom();
    i18n.init("en");
    showToast = vi.fn();
    settings = initTranslationSettings({ showToast });
  });

  it("只从普通配置读写翻译字段，不返回 API key", () => {
    settings.fill({
      ...configWith("openai_compatible", {
        endpoint: "https://translate.example.com/v1",
        model: "small-model",
      }),
      translation_target_language: "zh",
    });
    element("translation-api-key-input").value = "secret-value";

    const saved = settings.getConfig();
    expect(saved.translation_source_language).toBe("auto");
    expect(saved.translation_target_language).toBe("zh");
    expect(saved.translation_services).toHaveLength(6);
    expect(serviceOf(saved, "openai_compatible")).toEqual({
      provider: "openai_compatible",
      enabled: true,
      endpoint: "https://translate.example.com/v1",
      model: "small-model",
      region: "",
      project: "",
    });
    expect(saved.translation_services.filter((service) => service.enabled))
      .toHaveLength(1);
    expect(saved).not.toHaveProperty("translation_api_key");
    expect(JSON.stringify(saved)).not.toContain("secret-value");
  });

  it("切换 provider 时保留各服务自己的配置并查询对应密钥", async () => {
    hasTranslationApiKey.mockResolvedValue(false);
    settings.fill(configWith("libretranslate", { endpoint: "https://libre.example.com" }));

    const provider = element("translation-provider-select");
    provider.value = "openai_compatible";
    provider.dispatchEvent(new Event("change"));

    await vi.waitFor(() => {
      expect(hasTranslationApiKey).toHaveBeenCalledWith("openai_compatible");
    });
    // 端点留空表示沿用内置默认值，默认值只作为占位符出现。
    expect(element("translation-endpoint-input").value).toBe("");
    expect(element("translation-endpoint-input").placeholder).toBe("https://api.openai.com/v1");
    expect(element("translation-model-input").placeholder).toBe("gpt-4o-mini");
    // 选择器只切换“正在编辑哪个服务”，目的地摘要仍然只列启用的服务。
    expect(element("translation-service-enabled").checked).toBe(false);
    expect(element("translation-service-name").textContent).toBe("LibreTranslate-compatible");

    element("translation-model-input").value = "small-model";
    provider.value = "libretranslate";
    provider.dispatchEvent(new Event("change"));
    expect(element("translation-endpoint-input").value).toBe("https://libre.example.com");

    const saved = settings.getConfig();
    expect(serviceOf(saved, "openai_compatible").model).toBe("small-model");
    expect(serviceOf(saved, "openai_compatible").enabled).toBe(false);
    expect(serviceOf(saved, "libretranslate").enabled).toBe(true);
  });

  it("可以同时启用多个服务，目的地摘要按配置顺序列出全部启用项", () => {
    settings.fill(configWith("libretranslate", { endpoint: "https://libre.example.com" }));
    const provider = element("translation-provider-select");
    const enabled = element("translation-service-enabled");
    expect(enabled.checked).toBe(true);

    provider.value = "deepl";
    provider.dispatchEvent(new Event("change"));
    enabled.checked = true;
    enabled.dispatchEvent(new Event("change"));

    expect(element("translation-service-name").textContent)
      .toBe("LibreTranslate-compatible, DeepL");
    // 端点通过 title 暴露：并行翻译时用户需要知道文本会发往哪些地址。
    expect(element("translation-service-name").title)
      .toBe("https://libre.example.com\nhttps://api-free.deepl.com");

    const saved = settings.getConfig();
    expect(saved.translation_services.filter((service) => service.enabled)
      .map((service) => service.provider)).toEqual(["libretranslate", "deepl"]);
  });

  it("关闭最后一个服务后明确提示没有启用任何服务", () => {
    settings.fill(configWith("libretranslate"));
    const enabled = element("translation-service-enabled");
    enabled.checked = false;
    enabled.dispatchEvent(new Event("change"));

    expect(element("translation-service-name").textContent).toBe("No service enabled");
    expect(settings.getConfig().translation_services.some((service) => service.enabled))
      .toBe(false);
  });

  it("备选语言按勾选先后保存，一个都不勾时返回空列表", () => {
    settings.fill(configWith("libretranslate"));
    // 空列表表示沿用「目标 + 源」这一对语言，后端不需要额外标记。
    expect(settings.getConfig().preferred_languages).toEqual([]);

    for (const code of ["zh", "en"]) {
      preferred(code).checked = true;
      preferred(code).dispatchEvent(new Event("change"));
    }
    // 先勾的优先级更高，保存顺序不跟随勾选框的界面顺序。
    expect(settings.getConfig().preferred_languages).toEqual(["zh", "en"]);

    preferred("zh").checked = false;
    preferred("zh").dispatchEvent(new Event("change"));
    expect(settings.getConfig().preferred_languages).toEqual(["en"]);
  });

  it("回填备选语言时保留已保存的优先级顺序，丢弃界面上没有的语言码", () => {
    settings.fill({
      ...configWith("libretranslate"),
      preferred_languages: ["ja", "en", "kl", "en"],
    });

    expect(preferred("ja").checked).toBe(true);
    expect(preferred("en").checked).toBe(true);
    expect(preferred("zh").checked).toBe(false);
    // 顺序不能被勾选框的界面顺序重排，否则保存一次就改变了优先级。
    expect(settings.getConfig().preferred_languages).toEqual(["ja", "en"]);
  });

  it("按服务能力显示区域、项目、第二凭据字段与非官方端点提示", () => {
    settings.fill(configWith("libretranslate"));
    expect(element("translation-model-field").hidden).toBe(true);
    expect(element("translation-fallback-hint").hidden).toBe(true);

    const provider = element("translation-provider-select");
    provider.value = "bing";
    provider.dispatchEvent(new Event("change"));
    expect(element("translation-region-field").hidden).toBe(false);
    expect(element("translation-project-field").hidden).toBe(true);
    expect(element("translation-fallback-hint").hidden).toBe(false);

    provider.value = "google";
    provider.dispatchEvent(new Event("change"));
    expect(element("translation-project-field").hidden).toBe(false);
    expect(element("translation-region-field").hidden).toBe(true);

    provider.value = "youdao";
    provider.dispatchEvent(new Event("change"));
    expect(element("translation-api-secret-input").hidden).toBe(false);
  });

  it("通过专用 IPC 保存密钥，随后清空密码输入且不回显", async () => {
    hasTranslationApiKey.mockResolvedValue(false);
    setTranslationApiKey.mockResolvedValue(undefined);
    settings.fill(configWith("openai_compatible"));
    await settings.loadKeyStatus();

    const input = element("translation-api-key-input");
    input.value = "sk-sensitive";
    input.dispatchEvent(new Event("input"));
    element("translation-key-save-btn").click();

    await vi.waitFor(() => {
      expect(setTranslationApiKey)
        .toHaveBeenCalledWith("openai_compatible", "sk-sensitive", undefined);
    });
    expect(input.value).toBe("");
    expect(element("translation-key-status-text").textContent)
      .toBe("A key is stored for this service");
    expect(document.body.textContent).not.toContain("sk-sensitive");
    expect(showToast).toHaveBeenCalledWith("API key saved securely");
  });

  it("双字段服务在第二个凭据填好前不允许保存，保存时一起提交", async () => {
    hasTranslationApiKey.mockResolvedValue(false);
    setTranslationApiKey.mockResolvedValue(undefined);
    settings.fill(configWith("youdao"));
    await settings.loadKeyStatus();

    const keyInput = element("translation-api-key-input");
    const secretInput = element("translation-api-secret-input");
    keyInput.value = "app-key";
    keyInput.dispatchEvent(new Event("input"));
    expect(element("translation-key-save-btn").disabled).toBe(true);

    secretInput.value = "app-secret";
    secretInput.dispatchEvent(new Event("input"));
    expect(element("translation-key-save-btn").disabled).toBe(false);
    element("translation-key-save-btn").click();

    await vi.waitFor(() => {
      expect(setTranslationApiKey).toHaveBeenCalledWith("youdao", "app-key", "app-secret");
    });
    expect(secretInput.value).toBe("");
    expect(document.body.textContent).not.toContain("app-secret");
  });

  it("删除密钥后更新状态", async () => {
    hasTranslationApiKey.mockResolvedValue(true);
    deleteTranslationApiKey.mockResolvedValue(undefined);
    settings.fill(configWith("libretranslate"));
    await settings.loadKeyStatus();

    element("translation-key-delete-btn").click();

    await vi.waitFor(() => {
      expect(deleteTranslationApiKey).toHaveBeenCalledWith("libretranslate");
    });
    expect(element("translation-key-status-text").textContent)
      .toBe("No key is stored for this service");
  });

  it("清空已保存的译文，失败时保留可再次点击的按钮", async () => {
    settings.fill(configWith("libretranslate"));
    clearTranslationHistory.mockResolvedValue(undefined);

    element("translation-history-clear-btn").click();

    await vi.waitFor(() => {
      expect(element("translation-history-status-text").textContent)
        .toBe("Saved translations removed");
    });
    expect(clearTranslationHistory).toHaveBeenCalledTimes(1);
    expect(showToast).toHaveBeenCalledWith("Saved translations removed");
    expect(element("translation-history-clear-btn").disabled).toBe(false);

    clearTranslationHistory.mockRejectedValue(new Error("database is locked"));
    element("translation-history-clear-btn").click();

    await vi.waitFor(() => {
      expect(element("translation-history-status-text").textContent)
        .toBe("Could not remove the saved translations");
    });
    // 失败后按钮必须重新可用，否则用户没法重试。
    expect(element("translation-history-clear-btn").disabled).toBe(false);
  });

  it("忽略 provider 切换前返回的陈旧密钥状态", async () => {
    let resolveLibre;
    hasTranslationApiKey.mockImplementation((provider) => {
      if (provider === "libretranslate") {
        return new Promise((resolve) => { resolveLibre = resolve; });
      }
      return Promise.resolve(false);
    });
    settings.fill(configWith("libretranslate"));
    const staleRequest = settings.loadKeyStatus();

    const provider = element("translation-provider-select");
    provider.value = "openai_compatible";
    provider.dispatchEvent(new Event("change"));
    await vi.waitFor(() => {
      expect(element("translation-key-status-text").textContent)
        .toBe("No key is stored for this service");
    });

    resolveLibre(true);
    await staleRequest;
    expect(element("translation-key-status-text").textContent)
      .toBe("No key is stored for this service");
  });

  it("仅允许 HTTPS 或本机 HTTP 端点", () => {
    settings.fill(configWith("libretranslate", { endpoint: "http://translate.example.com" }));
    const endpoint = element("translation-endpoint-input");

    expect(() => settings.getConfig()).toThrow("Use an HTTPS endpoint");
    expect(endpoint.getAttribute("aria-invalid")).toBe("true");
    expect(document.activeElement).toBe(endpoint);

    endpoint.value = "HTTPS://translate.example.com";
    expect(() => settings.getConfig()).toThrow("Use an HTTPS endpoint");

    endpoint.value = "http://127.0.0.1:5000";
    expect(serviceOf(settings.getConfig(), "libretranslate").endpoint)
      .toBe("http://127.0.0.1:5000");
  });

  it("未选中服务的非法端点同样被拒绝，并切回出问题的服务", () => {
    settings.fill(configWith("libretranslate", { endpoint: "https://libre.example.com" }));
    // 另一个服务此前存下了非法端点，切换回去之前也不能悄悄保存。
    const stored = configWith("libretranslate", { endpoint: "https://libre.example.com" });
    serviceOf(stored, "deepl").endpoint = "ftp://deepl.example.com";
    settings.fill(stored);

    expect(() => settings.getConfig()).toThrow("Use an HTTPS endpoint");
    expect(element("translation-provider-select").value).toBe("deepl");
    expect(element("translation-endpoint-input").value).toBe("ftp://deepl.example.com");
  });
});
