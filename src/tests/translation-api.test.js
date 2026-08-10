import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import {
  deleteTranslationApiKey,
  hasTranslationApiKey,
  setTranslationApiKey,
  translateClip,
} from "../js/api.js";

describe("translation IPC wrappers", () => {
  beforeEach(() => invoke.mockReset());

  it("uses the stable provider and apiKey argument names", () => {
    setTranslationApiKey("openai_compatible", "secret");
    expect(invoke).toHaveBeenCalledWith("set_translation_api_key", {
      provider: "openai_compatible",
      apiKey: "secret",
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
    });
  });
});
