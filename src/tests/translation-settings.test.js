import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.js", () => ({
  deleteTranslationApiKey: vi.fn(),
  hasTranslationApiKey: vi.fn(),
  setTranslationApiKey: vi.fn(),
}));

import {
  deleteTranslationApiKey,
  hasTranslationApiKey,
  setTranslationApiKey,
} from "../js/api.js";
import { initTranslationSettings } from "../js/translation-settings.js";
import * as i18n from "../i18n/i18n.js";

function setupDom() {
  document.body.innerHTML = `
    <select id="translation-provider-select">
      <option value="libretranslate">LibreTranslate-compatible</option>
      <option value="openai_compatible">OpenAI-compatible</option>
    </select>
    <input id="translation-endpoint-input">
    <input id="translation-model-input">
    <select id="translation-source-language-select">
      <option value="auto">Auto</option><option value="en">English</option>
    </select>
    <select id="translation-target-language-select">
      <option value="en">English</option><option value="zh">Chinese</option>
    </select>
    <input id="translation-api-key-input" type="password">
    <button id="translation-key-save-btn"></button>
    <button id="translation-key-delete-btn" hidden></button>
    <span id="translation-key-status-dot"></span>
    <span id="translation-key-status-text"></span>
    <strong id="translation-service-name"></strong>
  `;
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
      translation_provider: "openai_compatible",
      translation_endpoint: "https://translate.example.com/v1",
      translation_model: "small-model",
      translation_source_language: "auto",
      translation_target_language: "zh",
    });
    document.getElementById("translation-api-key-input").value = "secret-value";

    expect(settings.getConfig()).toEqual({
      translation_provider: "openai_compatible",
      translation_endpoint: "https://translate.example.com/v1",
      translation_model: "small-model",
      translation_source_language: "auto",
      translation_target_language: "zh",
    });
    expect(settings.getConfig()).not.toHaveProperty("translation_api_key");
  });

  it("切换 provider 时替换默认端点并查询对应密钥", async () => {
    hasTranslationApiKey.mockResolvedValue(false);
    settings.fill({ translation_provider: "libretranslate" });

    const provider = document.getElementById("translation-provider-select");
    provider.value = "openai_compatible";
    provider.dispatchEvent(new Event("change"));

    await vi.waitFor(() => {
      expect(hasTranslationApiKey).toHaveBeenCalledWith("openai_compatible");
    });
    expect(document.getElementById("translation-endpoint-input").value)
      .toBe("https://api.openai.com/v1");
    expect(document.getElementById("translation-model-input").value).toBe("gpt-4o-mini");
    expect(document.getElementById("translation-service-name").textContent)
      .toBe("OpenAI-compatible");
  });

  it("通过专用 IPC 保存密钥，随后清空密码输入且不回显", async () => {
    hasTranslationApiKey.mockResolvedValue(false);
    setTranslationApiKey.mockResolvedValue(undefined);
    settings.fill({ translation_provider: "openai_compatible" });
    await settings.loadKeyStatus();

    const input = document.getElementById("translation-api-key-input");
    input.value = "sk-sensitive";
    input.dispatchEvent(new Event("input"));
    document.getElementById("translation-key-save-btn").click();

    await vi.waitFor(() => {
      expect(setTranslationApiKey).toHaveBeenCalledWith("openai_compatible", "sk-sensitive");
    });
    expect(input.value).toBe("");
    expect(document.getElementById("translation-key-status-text").textContent)
      .toBe("A key is stored for this service");
    expect(document.body.textContent).not.toContain("sk-sensitive");
    expect(showToast).toHaveBeenCalledWith("API key saved securely");
  });

  it("删除密钥后更新状态", async () => {
    hasTranslationApiKey.mockResolvedValue(true);
    deleteTranslationApiKey.mockResolvedValue(undefined);
    settings.fill({ translation_provider: "libretranslate" });
    await settings.loadKeyStatus();

    document.getElementById("translation-key-delete-btn").click();

    await vi.waitFor(() => {
      expect(deleteTranslationApiKey).toHaveBeenCalledWith("libretranslate");
    });
    expect(document.getElementById("translation-key-status-text").textContent)
      .toBe("No key is stored for this service");
  });

  it("忽略 provider 切换前返回的陈旧密钥状态", async () => {
    let resolveLibre;
    hasTranslationApiKey.mockImplementation((provider) => {
      if (provider === "libretranslate") {
        return new Promise((resolve) => { resolveLibre = resolve; });
      }
      return Promise.resolve(false);
    });
    settings.fill({ translation_provider: "libretranslate" });
    const staleRequest = settings.loadKeyStatus();

    const provider = document.getElementById("translation-provider-select");
    provider.value = "openai_compatible";
    provider.dispatchEvent(new Event("change"));
    await vi.waitFor(() => {
      expect(document.getElementById("translation-key-status-text").textContent)
        .toBe("No key is stored for this service");
    });

    resolveLibre(true);
    await staleRequest;
    expect(document.getElementById("translation-key-status-text").textContent)
      .toBe("No key is stored for this service");
  });

  it("仅允许 HTTPS 或本机 HTTP 端点", () => {
    settings.fill({
      translation_provider: "libretranslate",
      translation_endpoint: "http://translate.example.com",
    });
    const endpoint = document.getElementById("translation-endpoint-input");

    expect(() => settings.getConfig()).toThrow("Use an HTTPS endpoint");
    expect(endpoint.getAttribute("aria-invalid")).toBe("true");
    expect(document.activeElement).toBe(endpoint);

    endpoint.value = "HTTPS://translate.example.com";
    expect(() => settings.getConfig()).toThrow("Use an HTTPS endpoint");

    endpoint.value = "http://127.0.0.1:5000";
    expect(settings.getConfig().translation_endpoint).toBe("http://127.0.0.1:5000");
  });
});
