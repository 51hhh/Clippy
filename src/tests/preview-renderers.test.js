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
import { copyText, getClipImage, getConfig, ocrAvailable, ocrImage } from "../js/api.ts";

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
    expect(image?.parentElement).toBe(contentEl);
    expect(ocr?.parentElement).toBe(contentEl);
    expect(ocr?.classList.contains("preview-ocr-result--hidden")).toBe(true);
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
    image.resolve("image-bytes");
    await success;
    expect(contentEl.querySelector("img, .preview-ocr-result")).toBeNull();

    current = true;
    const failure = deferred();
    vi.mocked(getClipImage).mockReturnValueOnce(failure.promise);
    const failedRender = renderers.renderImage({ id: 9, byte_size: 1024 });
    current = false;
    failure.reject(new Error("read failed"));
    await failedRender;
    expect(contentEl.textContent).toBe("");
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
    vi.mocked(ocrImage).mockReturnValue(ocrResult.promise);
    renderers = createPreviewRenderers({
      contentEl,
      badgeEl,
      metaEl: document.createElement("div"),
      getLibraries: () => ({}),
      isCurrentRender: () => current,
    });

    const render = renderers.renderImage({ id: 8, byte_size: 1024 });
    current = false;
    firstConfig.resolve({ ocr_enabled: true });
    await render;
    expect(ocrAvailable).not.toHaveBeenCalled();
    expect(ocrImage).not.toHaveBeenCalled();

    current = true;
    vi.mocked(getConfig).mockReset();
    vi.mocked(getConfig).mockResolvedValueOnce({ ocr_enabled: true });
    await renderers.renderImage({ id: 9, byte_size: 1024 });
    expect(ocrImage).toHaveBeenCalledWith(9);
    expect(getConfig).toHaveBeenCalledTimes(1);

    const resultConfig = deferred();
    vi.mocked(getConfig).mockImplementation(() => resultConfig.promise);
    ocrResult.resolve("recognized text");
    await flushPromises();
    current = false;
    resultConfig.resolve({ ocr_result_mode: "clipboard" });
    await flushPromises();

    expect(copyText).not.toHaveBeenCalled();
    expect(contentEl.querySelector(".preview-ocr-result pre").textContent)
      .toBe("action.ocrProcessing");
  });
});
