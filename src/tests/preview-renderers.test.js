import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  getClipImage: vi.fn(),
  ocrAvailable: vi.fn(),
  ocrImage: vi.fn(),
  getConfig: vi.fn(),
  fetchUrlMeta: vi.fn(),
  copyText: vi.fn(),
}));

vi.mock("../i18n/i18n.js", () => ({
  t: (key) => key,
}));

import { createPreviewRenderers } from "../js/preview/renderers.js";

const RENDERER_NAMES = [
  "renderBase64Image", "renderCode", "renderColor", "renderCoordinate",
  "renderCron", "renderDataSize", "renderDate", "renderEmail",
  "renderEncoded", "renderEncrypted", "renderGradient", "renderHash",
  "renderHttpStatus", "renderImage", "renderIpAddress", "renderJson",
  "renderJwt", "renderMac", "renderMarkdown", "renderMathExpr",
  "renderMimeType", "renderNumberBase", "renderPlainText", "renderRegex",
  "renderRichText", "renderSemver", "renderTimestamp", "renderUrlCard",
  "renderUuid",
];

let contentEl;
let badgeEl;
let renderers;

beforeEach(() => {
  contentEl = document.createElement("div");
  badgeEl = document.createElement("div");
  renderers = createPreviewRenderers({
    contentEl,
    badgeEl,
    metaEl: document.createElement("div"),
    getLibraries: () => ({}),
    isCurrentClip: () => true,
  });
});

describe("preview renderer contract", () => {
  it("exposes every renderer used by the preview dispatcher", () => {
    expect(Object.keys(renderers).sort()).toEqual(RENDERER_NAMES);
  });

  it("renders plain user text without creating elements", () => {
    const text = '<img src=x onerror="alert(1)">';
    renderers.renderPlainText(text);

    expect(contentEl.textContent).toBe(text);
    expect(contentEl.querySelector("img")).toBeNull();
    expect(badgeEl.textContent).toBe("TEXT");
  });

  it("renders decoded and original values through text nodes", () => {
    renderers.renderEncoded({
      type: "base64",
      decoded: "<script>decoded</script>",
      original: '<img src=x onerror="alert(1)">',
    });

    const boxes = contentEl.querySelectorAll(".encoded-box");
    expect(boxes[0].textContent).toBe("<script>decoded</script>");
    expect(boxes[1].textContent).toBe('<img src=x onerror="alert(1)">');
    expect(contentEl.querySelector("script, img")).toBeNull();
  });
});
