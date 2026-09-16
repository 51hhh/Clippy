import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  getClipImage: vi.fn(),
  detectImageCodes: vi.fn(),
  ocrAvailable: vi.fn(),
  ocrImageResult: vi.fn(),
  getConfig: vi.fn(),
  fetchUrlMeta: vi.fn(),
  copyText: vi.fn(),
  openImageViewer: vi.fn(),
}));

vi.mock("../i18n/i18n.js", () => ({
  t: (key) => key,
}));

import { createPreviewRenderers } from "../js/preview/renderers.js";
import {
  copyText,
  detectImageCodes,
  getClipImage,
  getConfig,
  ocrAvailable,
  ocrImageResult,
} from "../js/api.ts";

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

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  vi.resetAllMocks();
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

  it("puts the image before OCR in the outer preview content flow", async () => {
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockResolvedValue({ ocr_enabled: false });

    await renderers.renderImage({ id: 8, byte_size: 1024 });

    const image = contentEl.querySelector("img");
    const ocr = contentEl.querySelector(".preview-ocr-result");
    expect(image).not.toBeNull();
    expect(ocr).not.toBeNull();
    expect(image?.compareDocumentPosition(ocr)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(image?.closest(".preview-image-card")?.parentElement).toBe(contentEl);
    expect(ocr?.parentElement).toBe(contentEl);
    expect(ocr?.dataset.status).toBe("disabled");
    expect(ocr?.querySelector(".preview-ocr-retry").hidden).toBe(false);
  });

  it("keeps sidebar scanning absent without invoking the retained scan API", async () => {
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockResolvedValue({ ocr_enabled: false });
    await renderers.renderImage({ id: 8, byte_size: 1024 });
    expect(contentEl.querySelector(".preview-code-scan")).toBeNull();
    expect(contentEl.querySelector(".preview-code-scan-button")).toBeNull();
    expect(detectImageCodes).not.toHaveBeenCalled();
    expect(contentEl.querySelector(".preview-image-open img")).not.toBeNull();
  });

  it("does not let a stale image success or failure replace the current preview", async () => {
    const image = deferred();
    let current = true;
    vi.mocked(getClipImage).mockReturnValue(image.promise);
    renderers = createPreviewRenderers({
      contentEl,
      badgeEl,
      metaEl: document.createElement("div"),
      getLibraries: () => ({}),
      isCurrentRender: () => current,
    });

    const success = renderers.renderImage({ id: 8, byte_size: 1024 });
    current = false;
    contentEl.textContent = "Current text preview";
    image.resolve("image-bytes");
    await success;
    expect(contentEl.querySelector("img, .preview-ocr-result")).toBeNull();
    expect(contentEl.textContent).toBe("Current text preview");

    current = true;
    contentEl.replaceChildren();
    const failure = deferred();
    vi.mocked(getClipImage).mockReturnValueOnce(failure.promise);
    const failedRender = renderers.renderImage({ id: 9, byte_size: 1024 });
    current = false;
    contentEl.textContent = "New preview after failed image";
    failure.reject(new Error("read failed"));
    await failedRender;
    expect(contentEl.textContent).toBe("New preview after failed image");
  });

  it("does not let a stale image onload overwrite metadata", async () => {
    const metaEl = document.createElement("div");
    let current = true;
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockResolvedValue({ ocr_enabled: false });
    renderers = createPreviewRenderers({
      contentEl,
      badgeEl,
      metaEl,
      getLibraries: () => ({}),
      isCurrentRender: () => current,
    });

    await renderers.renderImage({ id: 8, byte_size: 2048 });
    current = false;
    contentEl.querySelector("img").dispatchEvent(new Event("load"));
    expect(metaEl.textContent).toBe("");
  });

  it("does not start or publish stale OCR, including delayed clipboard configuration", async () => {
    const firstConfig = deferred();
    const ocrResult = deferred();
    let current = true;
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockReturnValueOnce(firstConfig.promise);
    vi.mocked(ocrAvailable).mockResolvedValue(true);
    vi.mocked(ocrImageResult).mockReturnValue(ocrResult.promise);
    renderers = createPreviewRenderers({
      contentEl,
      badgeEl,
      metaEl: document.createElement("div"),
      getLibraries: () => ({}),
      isCurrentRender: () => current,
    });

    const render = renderers.renderImage({ id: 8, byte_size: 1024 });
    await vi.waitFor(() => expect(getConfig).toHaveBeenCalledTimes(1));
    current = false;
    firstConfig.resolve({ ocr_enabled: true });
    await render;
    expect(ocrAvailable).not.toHaveBeenCalled();
    expect(ocrImageResult).not.toHaveBeenCalled();

    current = true;
    contentEl.replaceChildren();
    vi.mocked(getConfig).mockReset();
    vi.mocked(getConfig).mockResolvedValueOnce({ ocr_enabled: true });
    await renderers.renderImage({ id: 9, byte_size: 1024 });
    await vi.waitFor(() => expect(ocrImageResult).toHaveBeenCalledWith(9));
    expect(getConfig).toHaveBeenCalledTimes(1);

    const resultConfig = deferred();
    vi.mocked(getConfig).mockImplementation(() => resultConfig.promise);
    ocrResult.resolve("recognized text");
    await vi.waitFor(() => expect(getConfig).toHaveBeenCalledTimes(2));
    current = false;
    resultConfig.resolve({ ocr_result_mode: "clipboard" });
    await flushPromises();

    expect(copyText).not.toHaveBeenCalled();
    expect(contentEl.querySelector(".preview-ocr-result pre").textContent)
      .toBe("action.ocrProcessing");
  });
});
