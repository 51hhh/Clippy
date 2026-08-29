import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import {
  deleteTranslationApiKey,
  hasTranslationApiKey,
  setTranslationApiKey,
  translateCaptureSelection,
  translateClip,
  translateText,
} from "../js/api.ts";

describe("translation IPC wrappers", () => {
  beforeEach(() => invoke.mockReset());

  it("uses the stable provider and apiKey argument names", () => {
    setTranslationApiKey("openai_compatible", "secret");
    expect(invoke).toHaveBeenCalledWith("set_translation_api_key", {
      provider: "openai_compatible",
      apiKey: "secret",
      apiSecret: null,
    });
  });

  it("forwards the second credential field for dual-field services", () => {
    setTranslationApiKey("youdao", "app-key", "app-secret");
    expect(invoke).toHaveBeenCalledWith("set_translation_api_key", {
      provider: "youdao",
      apiKey: "app-key",
      apiSecret: "app-secret",
    });
  });

  it("does not expose key contents when checking or deleting", () => {
    hasTranslationApiKey("libretranslate");
    deleteTranslationApiKey("libretranslate");
    expect(invoke).toHaveBeenNthCalledWith(1, "has_translation_api_key", {
      provider: "libretranslate",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "delete_translation_api_key", {
      provider: "libretranslate",
    });
  });

  it("lets the backend allocate request ids for clip translation", () => {
    translateClip(42);
    expect(invoke).toHaveBeenCalledWith("translate_clip", {
      id: 42,
      sourceLanguage: null,
      targetLanguage: null,
      requestId: null,
      providers: null,
    });
  });

  it("narrows a retry to a single service", () => {
    translateClip(42, ["deepl"]);
    expect(invoke).toHaveBeenCalledWith("translate_clip", {
      id: 42,
      sourceLanguage: null,
      targetLanguage: null,
      requestId: null,
      providers: ["deepl"],
    });
  });

  it("uses camelCase optional arguments for direct text translation", () => {
    translateText("hello");
    expect(invoke).toHaveBeenCalledWith("translate_text", {
      text: "hello",
      sourceLanguage: null,
      targetLanguage: null,
      requestId: null,
      providers: null,
    });
  });

  it("sends capture geometry without an image payload", () => {
    const selection = {
      sessionId: "capture-1",
      monitorId: 2,
      x: 10,
      y: 20,
      width: 300,
      height: 160,
    };
    translateCaptureSelection(selection);
    expect(invoke).toHaveBeenCalledWith("translate_capture_selection", {
      selection,
      sourceLanguage: null,
      targetLanguage: null,
      requestId: null,
    });
    expect(JSON.stringify(invoke.mock.calls[0])).not.toContain("png");
  });
});
