import { beforeEach, describe, expect, it, vi } from "vitest";

const { translateClip } = vi.hoisted(() => ({ translateClip: vi.fn() }));

vi.mock("../js/api.ts", () => ({ translateClip }));

import * as i18n from "../i18n/i18n.js";
import * as translationPanel from "../js/translation-panel.js";

const CONFIG = {
  translation_provider: "libretranslate",
  translation_endpoint: "https://translate.example.test",
  translation_target_language: "zh",
};

function installPanel(config = CONFIG) {
  document.body.innerHTML = `
    <section id="translation-panel" hidden>
      <strong id="translation-provider"></strong>
      <span id="translation-target"></span>
      <span id="translation-endpoint"></span>
      <button id="translation-action"></button>
      <p id="translation-privacy"></p>
      <p id="translation-sensitive" tabindex="-1" hidden>Protected locally</p>
      <p id="translation-status" hidden></p>
      <div id="translation-result" hidden>
        <span id="translation-detected"></span>
        <button id="translation-copy"></button>
        <pre id="translation-result-text"></pre>
      </div>
    </section>`;
  i18n.init("en");
  translationPanel.init(config);
}

function clip(id, contentType = "text", sensitive = false) {
  return { id, content_type: contentType, is_sensitive: sensitive };
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
}

describe("translation panel", () => {
  beforeEach(() => {
    translateClip.mockReset();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
    installPanel();
  });

  it("shows the configured destination without starting a request", () => {
    translationPanel.updateClip(clip(11));

    expect(document.getElementById("translation-provider").textContent)
      .toBe("LibreTranslate-compatible");
    expect(document.getElementById("translation-target").textContent).toBe("Target: Chinese");
    expect(document.getElementById("translation-endpoint").textContent)
      .toBe("https://translate.example.test");
    expect(document.getElementById("translation-action").textContent).toBe("Translate");
    expect(translateClip).not.toHaveBeenCalled();
  });

  it("uses the image-specific action and requests translation only after a click", async () => {
    translateClip.mockResolvedValue({
      translated_text: "Invoice total",
      detected_source_language: "de",
    });
    translationPanel.updateClip(clip(12, "image"));

    expect(document.getElementById("translation-action").textContent).toBe("OCR & Translate");
    expect(translateClip).not.toHaveBeenCalled();

    document.getElementById("translation-action").click();
    await flushPromises();

    expect(translateClip).toHaveBeenCalledOnce();
    expect(translateClip).toHaveBeenCalledWith(12);
    expect(document.getElementById("translation-result-text").textContent).toBe("Invoice total");
    expect(document.getElementById("translation-detected").textContent).toBe("Detected source: de");
  });

  it("blocks sensitive items locally and explains the protection", () => {
    translationPanel.updateClip(clip(13, "text", true));

    const action = document.getElementById("translation-action");
    expect(action.disabled).toBe(true);
    expect(document.getElementById("translation-sensitive").hidden).toBe(false);
    action.click();
    expect(translateClip).not.toHaveBeenCalled();
  });

  it("moves keyboard focus to the action or local-protection explanation", () => {
    translationPanel.updateClip(clip(19));
    expect(translationPanel.focusAction()).toBe(true);
    expect(document.activeElement).toBe(document.getElementById("translation-action"));

    translationPanel.updateClip(clip(20, "text", true));
    expect(translationPanel.focusAction()).toBe(true);
    expect(document.activeElement).toBe(document.getElementById("translation-sensitive"));
  });

  it("discards a response after focus moves to another clip", async () => {
    let resolveRequest;
    translateClip.mockReturnValue(new Promise((resolve) => { resolveRequest = resolve; }));
    translationPanel.updateClip(clip(14));
    document.getElementById("translation-action").click();

    translationPanel.updateClip(clip(15));
    resolveRequest({ translated_text: "stale", detected_source_language: "en" });
    await flushPromises();

    expect(document.getElementById("translation-result").hidden).toBe(true);
    expect(document.getElementById("translation-result-text").textContent).toBe("");
    expect(document.getElementById("translation-action").textContent).toBe("Translate");
  });

  it("maps stable error codes without exposing backend details", async () => {
    translateClip.mockRejectedValue(
      "translation.network: connection failed for private-host.internal",
    );
    translationPanel.updateClip(clip(16));
    document.getElementById("translation-action").click();
    await flushPromises();

    const status = document.getElementById("translation-status").textContent;
    expect(status).toBe("Could not reach the translation service");
    expect(status).not.toContain("private-host.internal");
  });

  it("uses a generic message for errors without a stable code", async () => {
    translateClip.mockRejectedValue("worker failed with /home/user/private.txt");
    translationPanel.updateClip(clip(17));
    document.getElementById("translation-action").click();
    await flushPromises();

    expect(document.getElementById("translation-status").textContent)
      .toBe("Translation is temporarily unavailable");
  });

  it("copies the rendered result through navigator.clipboard", async () => {
    translateClip.mockResolvedValue({ translated_text: "Copied result" });
    translationPanel.updateClip(clip(18));
    document.getElementById("translation-action").click();
    await flushPromises();
    document.getElementById("translation-copy").click();
    await flushPromises();

    expect(navigator.clipboard.writeText).toHaveBeenCalledOnce();
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("Copied result");
  });
});

describe("translation error contract", () => {
  it("maps only prefixed stable codes", () => {
    expect(translationPanel.__test__.errorTranslationKey("translation.timeout: hidden"))
      .toBe("translation.error.timeout");
    expect(translationPanel.__test__.errorTranslationKey("timeout: hidden"))
      .toBe("translation.error.generic");
    expect(translationPanel.__test__.errorTranslationKey("translation.unknown: hidden"))
      .toBe("translation.error.generic");
  });
});

describe("translation localization", () => {
  it("has a complete Chinese action and protection vocabulary", () => {
    i18n.init("zh-CN");
    expect(i18n.t("translation.translate")).toBe("翻译");
    expect(i18n.t("translation.ocrAndTranslate")).toBe("OCR 并翻译");
    expect(i18n.t("translation.sensitive")).toContain("敏感剪贴板条目");
    expect(i18n.t("translation.error.network")).toContain("无法连接");
  });
});
