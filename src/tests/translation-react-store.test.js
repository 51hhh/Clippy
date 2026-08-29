import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  copyText: vi.fn(),
  speakClip: vi.fn(),
  speakText: vi.fn(),
  translateClip: vi.fn(),
  translationHistory: vi.fn(),
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

function service(provider, enabled, endpoint = "") {
  return { provider, enabled, endpoint, model: "", region: "", project: "" };
}

const config = {
  translation_services: [
    service("libretranslate", true, "https://translate.example.test"),
    service("deepl", false),
  ],
  translation_target_language: "zh",
};

/** 同时启用两个服务，用于多结果卡与单服务重试 */
const twoServices = {
  ...config,
  translation_services: [
    service("libretranslate", true, "https://translate.example.test"),
    service("deepl", true),
  ],
};

/** 默认目标语言与 config 一致，卡片上就不会出现换向提示 */
function ok(provider, translatedText, detected = null, targetLanguage = "zh") {
  return {
    status: "ok",
    provider,
    translated_text: translatedText,
    detected_source_language: detected,
    target_language: targetLanguage,
  };
}

function failed(provider, code) {
  return { status: "error", provider, code };
}

function batch(...services) {
  return { request_id: 1, services };
}

function cardOf(store, provider) {
  return store.getSnapshot().cards.find((card) => card.provider === provider);
}

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

/** jsdom 播不了音频，所以播放器是注入的：记录播过什么、能否失败即可 */
function fakePlayer() {
  return { play: vi.fn().mockResolvedValue(undefined), stop: vi.fn() };
}

const spoken = { mime_type: "audio/mpeg", audio_base64: "SUQz" };

describe("React translation store", () => {
  let store;
  let player;
  beforeEach(() => {
    i18n.init("en");
    player = fakePlayer();
    // 防抖窗口取 0：这些用例关心的是"查不查"，不是"等多久"
    store = new TranslationStore(player, 0);
    // 历史回填只在预览面板可见时发生，绝大多数用例都以面板已打开为前提
    store.setPanelVisible(true);
    api.translateClip.mockReset();
    api.copyText.mockReset();
    api.translationHistory.mockReset();
    api.translationHistory.mockResolvedValue([]);
    api.speakClip.mockReset();
    api.speakText.mockReset();
    store.setConfig(config);
  });

  it("translates only an explicit non-sensitive request and copies through IPC", async () => {
    store.setClip(clip());
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "你好", "en")));
    api.copyText.mockResolvedValue(undefined);
    await store.translate();
    expect(store.getSnapshot()).toMatchObject({ feedback: "complete", loading: false });
    expect(cardOf(store, "libretranslate")).toMatchObject({
      translatedText: "你好",
      detectedLanguage: "en",
      errorCode: null,
    });

    await store.copy("libretranslate");
    expect(api.copyText).toHaveBeenCalledWith("你好");
    expect(cardOf(store, "libretranslate").copyFeedback).toBe("copied");
  });

  it("shows saved translations for a clip without contacting a service", async () => {
    api.translationHistory.mockResolvedValue([
      {
        id: 9,
        clip_id: 4,
        provider: "libretranslate",
        source_language: "auto",
        target_language: "zh",
        source_text: "hello",
        translated_text: "早先的译文",
        created_at: 100,
      },
      // 未启用的服务留在库里的记录不该出现在界面上。
      {
        id: 8,
        clip_id: 4,
        provider: "deepl",
        source_language: "en",
        target_language: "zh",
        source_text: "hello",
        translated_text: "DeepL 译文",
        created_at: 90,
      },
    ]);
    store.setClip(clip(4));

    await vi.waitFor(() => {
      expect(store.getSnapshot().cards).toHaveLength(1);
    });
    expect(api.translationHistory).toHaveBeenCalledWith(4);
    expect(api.translateClip).not.toHaveBeenCalled();
    expect(cardOf(store, "libretranslate")).toMatchObject({
      translatedText: "早先的译文",
      // 记录里的 auto 表示服务当时没报告检测结果。
      detectedLanguage: null,
      fromHistory: true,
    });
    // 这不是本次翻译的结果，汇总行不能显示成"翻译完成"。
    expect(store.getSnapshot().feedback).toBe("idle");

    const html = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(html).toContain("Saved earlier");
    expect(html).toContain("早先的译文");
    expect(html).not.toContain("DeepL 译文");

    // 重新翻译后卡片是新结果，不再标注为保存的译文。
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "新的译文", "en")));
    await store.translate();
    expect(cardOf(store, "libretranslate")).toMatchObject({
      translatedText: "新的译文",
      fromHistory: false,
    });
  });

  it("does not query saved translations while the preview panel is hidden", async () => {
    const hidden = new TranslationStore(fakePlayer(), 0);
    hidden.setConfig(config);
    hidden.setClip(clip(4));
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(api.translationHistory).not.toHaveBeenCalled();

    // 打开面板才去查，并且查的是当前条目
    hidden.setPanelVisible(true);
    await vi.waitFor(() => {
      expect(api.translationHistory).toHaveBeenCalledWith(4);
    });
  });

  it("debounces saved-translation lookups to the clip the user stops on", async () => {
    const debounced = new TranslationStore(fakePlayer(), 40);
    debounced.setPanelVisible(true);
    debounced.setConfig(config);
    debounced.setClip(clip(1));
    debounced.setClip(clip(2));
    debounced.setClip(clip(3));
    await vi.waitFor(() => {
      expect(api.translationHistory).toHaveBeenCalledTimes(1);
    });
    expect(api.translationHistory).toHaveBeenCalledWith(3);
  });

  it("keeps saved translations out of the panel for sensitive items", async () => {
    api.translationHistory.mockResolvedValue([
      {
        id: 7,
        clip_id: 5,
        provider: "libretranslate",
        source_language: "en",
        target_language: "zh",
        source_text: "hello",
        translated_text: "不该出现",
        created_at: 100,
      },
    ]);
    store.setClip(clip(5, true));
    await Promise.resolve();
    expect(api.translationHistory).not.toHaveBeenCalled();
    expect(store.getSnapshot().cards).toHaveLength(0);
  });

  it("plays the clip text and a card translation through the backend audio", async () => {
    store.setClip(clip());
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "你好", "en")));
    await store.translate();
    api.speakClip.mockResolvedValue(spoken);
    api.speakText.mockResolvedValue(spoken);

    await store.speakSource();
    expect(api.speakClip).toHaveBeenCalledWith(1);
    expect(player.play).toHaveBeenCalledWith(spoken);
    // 播完后按钮恢复可用，不停留在"正在播放"。
    expect(store.getSnapshot()).toMatchObject({ speaking: null, speechErrorCode: null });

    await store.speakTranslation("libretranslate");
    // 译文按后端实际使用的目标语言发音。
    expect(api.speakText).toHaveBeenCalledWith("你好", "zh");
    expect(player.play).toHaveBeenCalledTimes(2);
  });

  it("keeps sensitive items and empty cards out of the audio path", async () => {
    store.setClip(clip(2, true));
    await store.speakSource();
    expect(api.speakClip).not.toHaveBeenCalled();

    store.setClip(clip(3));
    // 还没有译文的卡不能朗读。
    await store.speakTranslation("libretranslate");
    expect(api.speakText).not.toHaveBeenCalled();
  });

  it("reports a failed playback without discarding the translation", async () => {
    store.setClip(clip());
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "你好", "en")));
    await store.translate();
    api.speakText.mockRejectedValue(new Error("translation.network: unreachable"));

    await store.speakTranslation("libretranslate");
    expect(store.getSnapshot()).toMatchObject({
      speaking: null,
      speechErrorCode: "network",
      feedback: "complete",
    });
    expect(cardOf(store, "libretranslate").translatedText).toBe("你好");

    const html = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(html).toContain("Could not reach the translation service");

    // 切换条目会停掉在播的音频并清掉上一条提示。
    store.setClip(clip(4));
    expect(player.stop).toHaveBeenCalled();
    expect(store.getSnapshot().speechErrorCode).toBeNull();
  });

  it("ignores a second play request while audio is still playing", async () => {
    store.setClip(clip());
    const pending = deferred();
    api.speakClip.mockReturnValue(pending.promise);

    const first = store.speakSource();
    expect(store.getSnapshot().speaking).toBe("source");
    await store.speakSource();
    expect(api.speakClip).toHaveBeenCalledTimes(1);

    pending.resolve(spoken);
    await first;
    expect(store.getSnapshot().speaking).toBeNull();
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

  it("lists every enabled destination, using built-in endpoints where none is set", () => {
    store.setConfig(twoServices);
    store.setClip(clip(3));
    const html = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(html).toContain("https://translate.example.test");
    expect(html).toContain("https://api-free.deepl.com");
    expect(html).toContain("DeepL");
  });

  it("says no service is enabled instead of naming a default one", () => {
    store.setConfig({
      ...config,
      translation_services: config.translation_services.map((entry) => ({
        ...entry,
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

  it("names the language a card was actually translated into after an auto switch", async () => {
    store.setClip(clip(16));
    // 后端发现文本本来就是目标语言（中文）时会换向，界面必须按实际目标展示。
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "Hello", "zh", "en")));
    await store.translate();
    expect(cardOf(store, "libretranslate").targetLanguage).toBe("en");

    const html = renderToStaticMarkup(React.createElement(TranslationPanel, { store }));
    expect(html).toContain("Target: Chinese");
    expect(html).toContain("Target: English");
  });

  it("keeps one card per service and reports a partly failed batch", async () => {
    store.setConfig(twoServices);
    store.setClip(clip(9));
    api.translateClip.mockResolvedValue(batch(
      ok("libretranslate", "你好"),
      failed("deepl", "rate_limited"),
    ));
    await store.translate();

    expect(store.getSnapshot().cards.map((card) => card.provider))
      .toEqual(["libretranslate", "deepl"]);
    expect(store.getSnapshot()).toMatchObject({ feedback: "partial", errorCode: null });
    expect(cardOf(store, "deepl")).toMatchObject({
      errorCode: "rate_limited",
      translatedText: "",
    });
  });

  it("shows placeholder cards for every enabled service while the batch runs", async () => {
    store.setConfig(twoServices);
    store.setClip(clip(10));
    const request = deferred();
    api.translateClip.mockReturnValue(request.promise);
    const translating = store.translate();

    expect(store.getSnapshot().cards).toHaveLength(2);
    expect(store.getSnapshot().cards.every((card) => card.loading)).toBe(true);

    request.resolve(batch(ok("libretranslate", "你好"), ok("deepl", "Hallo")));
    await translating;
    expect(store.getSnapshot()).toMatchObject({ feedback: "complete" });
  });

  it("retries a single service and keeps the other card untouched", async () => {
    store.setConfig(twoServices);
    store.setClip(clip(11));
    api.translateClip.mockResolvedValueOnce(batch(
      ok("libretranslate", "你好"),
      failed("deepl", "timeout"),
    ));
    await store.translate();

    api.translateClip.mockResolvedValueOnce(batch(ok("deepl", "Hallo", "en")));
    await store.retry("deepl");

    expect(api.translateClip).toHaveBeenLastCalledWith(11, ["deepl"]);
    expect(cardOf(store, "libretranslate").translatedText).toBe("你好");
    expect(cardOf(store, "deepl")).toMatchObject({ translatedText: "Hallo", errorCode: null });
    expect(store.getSnapshot()).toMatchObject({ feedback: "complete" });
  });

  it("does not start a retry while the batch is still running", async () => {
    store.setConfig(twoServices);
    store.setClip(clip(12));
    const request = deferred();
    api.translateClip.mockReturnValue(request.promise);
    const translating = store.translate();

    await store.retry("deepl");
    expect(api.translateClip).toHaveBeenCalledTimes(1);

    request.resolve(batch(ok("libretranslate", "你好"), ok("deepl", "Hallo")));
    await translating;
  });

  it("drops stale translation responses after focus changes", async () => {
    const request = deferred();
    api.translateClip.mockReturnValue(request.promise);
    store.setClip(clip(3));
    const translating = store.translate();
    store.setClip(clip(4));
    request.resolve(batch(ok("libretranslate", "stale")));
    await translating;
    expect(store.getSnapshot().cards).toEqual([]);
  });

  it("drops an in-flight response when translation settings change", async () => {
    const request = deferred();
    api.translateClip.mockReturnValue(request.promise);
    store.setClip(clip(5));
    const translating = store.translate();
    store.setConfig({ ...config, translation_target_language: "en" });
    request.resolve(batch(ok("libretranslate", "stale", "de")));
    await translating;
    expect(store.getSnapshot()).toMatchObject({
      cards: [],
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
    expect(translationFeedbackText("partial", null))
      .toBe("Some services could not translate");
    i18n.init("zh-CN");
    expect(translationFeedbackText("error", "network")).toContain("无法连接");
  });

  it("reports a request-level failure without leaving service cards behind", async () => {
    store.setClip(clip(13));
    api.translateClip.mockRejectedValue(new Error("translation.no_service_enabled: hidden"));
    await store.translate();
    expect(store.getSnapshot()).toMatchObject({
      cards: [],
      feedback: "error",
      errorCode: "no_service_enabled",
    });
  });

  it("maps an empty service result to the stable invalid-response error", async () => {
    store.setClip(clip(6));
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "   ")));
    await store.translate();
    expect(store.getSnapshot()).toMatchObject({
      feedback: "error",
      errorCode: "invalid_response",
    });
    expect(cardOf(store, "libretranslate").errorCode).toBe("invalid_response");
  });

  it("degrades an unknown service error code to the generic message", async () => {
    store.setClip(clip(14));
    api.translateClip.mockResolvedValue(batch(failed("libretranslate", "code_from_a_newer_build")));
    await store.translate();
    // 认不出的码退化为 internal（通用提示），但这张卡仍然算失败。
    expect(cardOf(store, "libretranslate").errorCode).toBe("internal");
    expect(store.getSnapshot().feedback).toBe("error");
  });

  it("does not apply copy feedback after focus changes", async () => {
    const request = deferred();
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "result")));
    api.copyText.mockReturnValue(request.promise);
    store.setClip(clip(7));
    await store.translate();

    const copying = store.copy("libretranslate");
    store.setClip(clip(8));
    request.resolve(undefined);
    await copying;

    expect(store.getSnapshot()).toMatchObject({ feedback: "idle", cards: [] });
  });

  it("shows copy feedback on one card only", async () => {
    store.setConfig(twoServices);
    store.setClip(clip(15));
    api.translateClip.mockResolvedValue(batch(ok("libretranslate", "你好"), ok("deepl", "Hallo")));
    api.copyText.mockResolvedValue(undefined);
    await store.translate();

    await store.copy("libretranslate");
    await store.copy("deepl");
    expect(cardOf(store, "libretranslate").copyFeedback).toBe("idle");
    expect(cardOf(store, "deepl").copyFeedback).toBe("copied");
    expect(api.copyText).toHaveBeenLastCalledWith("Hallo");
  });
});
