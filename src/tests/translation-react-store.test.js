import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  copyText: vi.fn(),
  translateClip: vi.fn(),
}));

import * as api from "../js/api.ts";
import * as i18n from "../i18n/i18n.js";
import { TranslationPanel, translationFeedbackText } from "../react/main/TranslationPanel.tsx";
import {
  TranslationStore,
  stableTranslationErrorCode,
} from "../react/main/translationStore.ts";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

function clip(id = 1, sensitive = false, contentType = "text") {
  return {
    id,
    content_type: contentType,
    text_content: "hello",
    html_content: null,
    image_data: null,
    content_hash: String(id),
    is_favorite: false,
    is_sensitive: sensitive,
    created_at: 1,
    byte_size: 5,
  };
}

const config = {
  translation_services: [
    {
      provider: "libretranslate",
      enabled: true,
      endpoint: "https://translate.example.test",
      model: "",
      region: "",
      project: "",
    },
    {
      provider: "deepl",
      enabled: false,
      endpoint: "",
      model: "",
      region: "",
      project: "",
    },
  ],
  translation_target_language: "zh",
};

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

describe("React translation store", () => {
  let store;
  beforeEach(() => {
    i18n.init("en");
    store = new TranslationStore();
    api.translateClip.mockReset();
    api.copyText.mockReset();
    store.setConfig(config);
  });

  it("translates only an explicit non-sensitive request and copies through IPC", async () => {
    store.setClip(clip());
    api.translateClip.mockResolvedValue({
      translated_text: "你好",
      detected_source_language: "en",
    });
    api.copyText.mockResolvedValue(undefined);
    await store.translate();
    expect(store.getSnapshot()).toMatchObject({ translatedText: "你好", feedback: "complete" });
    await store.copy();
    expect(api.copyText).toHaveBeenCalledWith("你好");
    expect(store.getSnapshot().feedback).toBe("copied");
  });

  it("blocks sensitive items before calling the service", async () => {
    store.setClip(clip(2, true));
    await store.translate();
    expect(api.translateClip).not.toHaveBeenCalled();
  });

  it("renders the configured destination and image action without starting a request", () => {
    store.setClip(clip(2, false, "image"));
    const html = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(html).toContain("LibreTranslate-compatible");
    expect(html).toContain("Target: Chinese");
    expect(html).toContain("https://translate.example.test");
    expect(html).toContain("OCR &amp; Translate");
    expect(api.translateClip).not.toHaveBeenCalled();
  });

  it("says no service is enabled instead of naming a default one", () => {
    store.setConfig({
      ...config,
      translation_services: config.translation_services.map((service) => ({
        ...service,
        enabled: false,
      })),
    });
    store.setClip(clip(3));
    const html = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(html).toContain("No service enabled");
    expect(html).toContain("Endpoint not configured");
    expect(html).not.toContain("https://translate.example.test");
  });

  it("describes sensitive protection only when the explanation is rendered", () => {
    store.setClip(clip(7));
    const normalHtml = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(normalHtml).toContain('aria-describedby="translation-privacy-react"');
    expect(normalHtml).not.toContain("translation-sensitive-react");

    store.setClip(clip(8, true));
    const sensitiveHtml = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(sensitiveHtml).toContain(
      'aria-describedby="translation-privacy-react translation-sensitive-react"',
    );
    expect(sensitiveHtml).toContain('id="translation-sensitive-react"');
    expect(sensitiveHtml).toContain("disabled");
  });

  it("drops stale translation responses after focus changes", async () => {
    const request = deferred();
    api.translateClip.mockReturnValue(request.promise);
    store.setClip(clip(3));
    const translating = store.translate();
    store.setClip(clip(4));
    request.resolve({ translated_text: "stale", detected_source_language: "en" });
    await translating;
    expect(store.getSnapshot().translatedText).toBe("");
  });

  it("drops an in-flight response when translation settings change", async () => {
    const request = deferred();
    api.translateClip.mockReturnValue(request.promise);
    store.setClip(clip(5));
    const translating = store.translate();
    store.setConfig({ ...config, translation_target_language: "en" });
    request.resolve({ translated_text: "stale", detected_source_language: "de" });
    await translating;
    expect(store.getSnapshot()).toMatchObject({
      translatedText: "",
      loading: false,
      feedback: "idle",
    });
  });

  it("maps only stable backend error codes", () => {
    expect(stableTranslationErrorCode("translation.timeout: hidden")).toBe("timeout");
    expect(stableTranslationErrorCode(new Error("translation.invalid_response:")))
      .toBe("invalid_response");
    expect(stableTranslationErrorCode("worker failed with /private/path")).toBeNull();
  });

  it("localizes stable errors without exposing backend details", () => {
    expect(translationFeedbackText("error", "network"))
      .toBe("Could not reach the translation service");
    expect(translationFeedbackText("error", null))
      .toBe("Translation is temporarily unavailable");
    i18n.init("zh-CN");
    expect(translationFeedbackText("error", "network")).toContain("无法连接");
  });

  it("maps an empty service result to the stable invalid-response error", async () => {
    store.setClip(clip(6));
    api.translateClip.mockResolvedValue({
      translated_text: "   ",
      detected_source_language: null,
    });
    await store.translate();
    expect(store.getSnapshot()).toMatchObject({
      feedback: "error",
      errorCode: "invalid_response",
    });
  });

  it("does not apply copy feedback after focus changes", async () => {
    const request = deferred();
    api.translateClip.mockResolvedValue({ translated_text: "result" });
    api.copyText.mockReturnValue(request.promise);
    store.setClip(clip(7));
    await store.translate();

    const copying = store.copy();
    store.setClip(clip(8));
    request.resolve(undefined);
    await copying;

    expect(store.getSnapshot()).toMatchObject({ feedback: "idle", translatedText: "" });
  });
});
