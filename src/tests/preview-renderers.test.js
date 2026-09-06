import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  getClipImage: vi.fn(),
  detectImageCodes: vi.fn(),
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
import {
  copyText,
  detectImageCodes,
  getClipImage,
  getConfig,
  ocrAvailable,
  ocrImage,
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
    expect(image?.parentElement).toBe(contentEl);
    expect(ocr?.parentElement).toBe(contentEl);
    expect(ocr?.classList.contains("preview-ocr-result--hidden")).toBe(true);
  });

  it("only scans after an explicit click, renders untrusted code text safely, and copies per result", async () => {
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockResolvedValue({ ocr_enabled: false });
    vi.mocked(detectImageCodes).mockResolvedValue({
      results: [
        {
          format: "qr_code",
          text: '<a href="javascript:alert(1)">QR</a>\u0000\u202E',
          points: [{ x: 0, y: 0 }],
        },
        { format: "code_39", text: "CODE-39", points: [] },
      ],
      limited: true,
    });
    vi.mocked(copyText)
      .mockResolvedValueOnce()
      .mockRejectedValueOnce(new Error("clipboard unavailable"));

    await renderers.renderImage({ id: 8, byte_size: 1024 });

    const scanButton = contentEl.querySelector(".preview-code-scan-button");
    const area = contentEl.querySelector(".preview-code-scan");
    expect(scanButton).not.toBeNull();
    expect(detectImageCodes).not.toHaveBeenCalled();
    expect(area.dataset.status).toBe("idle");

    scanButton.click();
    expect(area.dataset.status).toBe("loading");
    expect(scanButton.disabled).toBe(true);
    scanButton.click();
    expect(detectImageCodes).toHaveBeenCalledTimes(1);
    await flushPromises();

    expect(detectImageCodes).toHaveBeenCalledWith(8);
    expect(area.dataset.status).toBe("limited");
    expect(area.querySelectorAll(".preview-code-scan-result")).toHaveLength(2);
    expect(area.querySelector(".preview-code-scan-text").textContent)
      .toBe('<a href="javascript:alert(1)">QR</a>\u0000\u202E');
    expect(area.querySelector("a, script, img")).toBeNull();

    area.querySelector(".preview-code-scan-copy").click();
    await flushPromises();
    expect(copyText).toHaveBeenCalledWith('<a href="javascript:alert(1)">QR</a>\u0000\u202E');
    expect(area.querySelector(".preview-code-scan-copy").textContent).toBe("codeScan.copied");

    const copyButtons = area.querySelectorAll(".preview-code-scan-copy");
    copyButtons[1].click();
    expect(copyButtons[1].disabled).toBe(true);
    await flushPromises();
    expect(copyText).toHaveBeenLastCalledWith("CODE-39");
    expect(copyButtons[1].textContent).toBe("codeScan.copyFailed");
    expect(copyButtons[1].disabled).toBe(false);
  });

  it("reports empty and complete-result scan states after explicit requests", async () => {
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockResolvedValue({ ocr_enabled: false });
    vi.mocked(detectImageCodes)
      .mockResolvedValueOnce({ results: [], limited: false })
      .mockResolvedValueOnce({
        results: [{ format: "code_39", text: "FOUND", points: [] }],
        limited: false,
      });

    await renderers.renderImage({ id: 17, byte_size: 1024 });
    const area = contentEl.querySelector(".preview-code-scan");
    const scanButton = area.querySelector(".preview-code-scan-button");
    scanButton.click();
    await flushPromises();
    expect(area.dataset.status).toBe("empty");

    scanButton.click();
    await flushPromises();
    expect(area.dataset.status).toBe("results");
    expect(area.querySelector(".preview-code-scan-status").textContent).toBe("codeScan.found");
  });

  it("keeps the image and OCR area when an explicit code scan fails", async () => {
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockResolvedValue({ ocr_enabled: false });
    vi.mocked(detectImageCodes).mockRejectedValue(new Error("malformed response"));

    await renderers.renderImage({ id: 18, byte_size: 1024 });
    contentEl.querySelector(".preview-code-scan-button").click();
    await flushPromises();

    expect(contentEl.querySelector("img")).not.toBeNull();
    expect(contentEl.querySelector(".preview-ocr-result")).not.toBeNull();
    expect(contentEl.querySelector(".preview-code-scan").dataset.status).toBe("error");
    expect(contentEl.querySelector(".preview-code-scan-status").textContent).toBe("codeScan.failed");
  });

  it("isolates scan failure from OCR and discards stale scan or copy completions", async () => {
    const scan = deferred();
    const copy = deferred();
    let current = true;
    vi.mocked(getClipImage).mockResolvedValue("image-bytes");
    vi.mocked(getConfig).mockResolvedValue({ ocr_enabled: true });
    vi.mocked(ocrAvailable).mockRejectedValue(new Error("OCR missing"));
    vi.mocked(detectImageCodes).mockReturnValue(scan.promise);
    vi.mocked(copyText).mockReturnValue(copy.promise);
    renderers = createPreviewRenderers({
      contentEl,
      badgeEl,
      metaEl: document.createElement("div"),
      getLibraries: () => ({}),
      isCurrentRender: () => current,
    });

    await renderers.renderImage({ id: 9, byte_size: 1024 });
    const area = contentEl.querySelector(".preview-code-scan");
    area.querySelector(".preview-code-scan-button").click();
    expect(area.dataset.status).toBe("loading");
    expect(contentEl.querySelector(".preview-ocr-result").dataset.status).toBe("unavailable");

    current = false;
    scan.resolve({ results: [{ format: "qr_code", text: "late", points: [] }], limited: false });
    await flushPromises();
    expect(area.dataset.status).toBe("loading");
    expect(area.querySelectorAll(".preview-code-scan-result")).toHaveLength(0);

    // 真实面板在切换后会清空旧代次 DOM；新一代才能接受下一次显式扫描。
    contentEl.replaceChildren();
    current = true;
    vi.mocked(detectImageCodes).mockResolvedValueOnce({
      results: [{ format: "qr_code", text: "copy later", points: [] }],
      limited: false,
    });
    await renderers.renderImage({ id: 10, byte_size: 1024 });
    const currentArea = contentEl.querySelector(".preview-code-scan");
    currentArea.querySelector(".preview-code-scan-button").click();
    await flushPromises();
    const copyButton = currentArea.querySelector(".preview-code-scan-copy");
    copyButton.click();
    current = false;
    copy.resolve();
    await flushPromises();
    expect(copyButton.textContent).toBe("action.copy");
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
