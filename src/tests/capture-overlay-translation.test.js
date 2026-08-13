import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as i18n from "../i18n/i18n.js";
import { TranslationPopover } from "../react/capture-overlay/TranslationPopover.tsx";
import {
  isCurrentTranslation,
  translationErrorMessage,
  translationPanelPosition,
} from "../react/capture-overlay/translationState.ts";

describe("capture overlay translation", () => {
  beforeEach(() => i18n.init("en"));

  it("renders provider, local OCR and translation as escaped accessible text", () => {
    const html = renderToStaticMarkup(React.createElement(TranslationPopover, {
      state: {
        status: "result",
        result: {
          requestId: 9,
          provider: "openai_compatible",
          sourceText: "<script>local source</script>",
          translatedText: "translated & safe",
          detectedSourceLanguage: "en",
        },
      },
      left: 8,
      top: 8,
      copyStatus: null,
      onCopy: vi.fn(),
      onClose: vi.fn(),
    }));

    expect(html).toContain('role="dialog"');
    expect(html).toContain('aria-label="Close translation"');
    expect(html).toContain("OpenAI compatible");
    expect(html).toContain("&lt;script&gt;local source&lt;/script&gt;");
    expect(html).toContain("translated &amp; safe");
    expect(html).not.toContain("<script>local source</script>");
  });

  it("maps only stable translation codes and hides unknown error details", () => {
    expect(translationErrorMessage(
      "translation.ocr_failed: private OCR process detail",
    )).toBe("Local OCR could not extract text from this selection.");
    expect(translationErrorMessage("secret backend stack trace"))
      .toBe("Translation failed.");
  });

  it("localizes stable translation errors without exposing backend details", () => {
    i18n.init("zh-CN");
    expect(translationErrorMessage("translation.timeout: private endpoint"))
      .toBe("翻译请求超时。");
    expect(translationErrorMessage("private backend stack trace")).toBe("翻译失败。");
  });

  it("rejects stale callbacks and keeps the popover within normal viewports", () => {
    expect(isCurrentTranslation(4, 3)).toBe(false);
    expect(isCurrentTranslation(4, 4)).toBe(true);

    const position = translationPanelPosition(
      { x: 940, y: 600, width: 80, height: 60 },
      1024,
      768,
    );
    expect(position.left).toBeGreaterThanOrEqual(8);
    expect(position.left + 360).toBeLessThanOrEqual(1016);
    expect(position.top).toBeGreaterThanOrEqual(8);
    expect(position.top + 320).toBeLessThanOrEqual(760);
  });
});
