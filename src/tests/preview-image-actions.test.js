import { beforeEach, describe, expect, it, vi } from "vitest";
import { createContentRenderers } from "../js/preview/content-renderers.js";
import { createImageTranslationView, revealImageTranslation } from "../js/preview/reveal-translation.js";
import { init } from "../i18n/i18n.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(overrides = {}) {
  const contentEl = document.createElement("div");
  const services = {
    getClipImage: vi.fn().mockResolvedValue("image-bytes"),
    detectImageCodes: vi.fn().mockResolvedValue({ results: [], limited: false }),
    ocrAvailable: vi.fn().mockResolvedValue(true),
    ocrImage: vi.fn().mockResolvedValue("<script>recognized text</script>"),
    getConfig: vi.fn().mockResolvedValue({ ocr_enabled: true, ocr_result_mode: "panel" }),
    fetchUrlMeta: vi.fn(),
    copyText: vi.fn().mockResolvedValue(undefined),
    openImageViewer: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
  let current = true;
  const imageTranslation = createImageTranslationView();
  const renderers = createContentRenderers({
    contentEl, services, imageTranslation,
    badgeEl: document.createElement("span"),
    metaEl: document.createElement("span"),
    getLibraries: () => ({}),
    isCurrentRender: () => current,
  });
  return { contentEl, services, renderers, imageTranslation, expire: () => { current = false; } };
}

const clip = { id: 23, byte_size: 1024 };
const status = (f) => f.contentEl.querySelector(".preview-ocr-result").dataset.status;

beforeEach(() => init("en"));

describe("image sidebar actions", () => {
  it("shows actual OCR provenance and keeps rejected raw lines out of visible/copied text", async () => {
    const f = fixture({ ocrImage: vi.fn().mockResolvedValue({
      text: "accepted text", lines: [{ text: "low confidence raw", accepted: false }],
      pipeline: { engine: "tesseract" }, fallbackReason: "Enhanced runtime is not configured",
    }) });
    await f.renderers.renderImage(clip);
    await vi.waitFor(() => expect(status(f)).toBe("done"));
    const provenance = f.contentEl.querySelector(".preview-ocr-provenance");
    expect(provenance.hidden).toBe(false);
    expect(provenance.textContent).toContain("Tesseract OCR");
    expect(provenance.querySelector("details p").textContent).toBe("Enhanced runtime is not configured");
    expect(f.contentEl.textContent).not.toContain("low confidence raw");
    f.contentEl.querySelector(".preview-ocr-copy").click();
    await vi.waitFor(() => expect(f.services.copyText).toHaveBeenCalledWith("accepted text"));
  });

  it("keeps OCR readable after automatic copy and allows explicit copying again", async () => {
    const f = fixture({ getConfig: vi.fn().mockResolvedValue({ ocr_enabled: true, ocr_result_mode: "clipboard" }) });
    await f.renderers.renderImage(clip);
    await vi.waitFor(() => expect(f.services.copyText).toHaveBeenCalledTimes(1));
    const text = f.contentEl.querySelector(".preview-ocr-result pre");
    expect(text.textContent).toBe("<script>recognized text</script>");
    expect(text.querySelector("script")).toBeNull();
    f.contentEl.querySelector(".preview-ocr-copy").click();
    await vi.waitFor(() => expect(f.services.copyText).toHaveBeenCalledTimes(2));
    expect(f.services.copyText).toHaveBeenLastCalledWith(text.textContent);
    expect(text.textContent).toBe("<script>recognized text</script>");
  });

  it("makes OCR retry explicit, prevents duplicate requests, and preserves text after a copy failure", async () => {
    const retry = deferred();
    const f = fixture({
      ocrImage: vi.fn().mockRejectedValueOnce(new Error("temporary OCR failure")).mockReturnValueOnce(retry.promise),
      copyText: vi.fn().mockRejectedValue(new Error("clipboard busy")),
    });
    await f.renderers.renderImage(clip);
    await vi.waitFor(() => expect(status(f)).toBe("error"));
    const retryButton = f.contentEl.querySelector(".preview-ocr-retry");
    const copyButton = f.contentEl.querySelector(".preview-ocr-copy");
    expect(retryButton.textContent).toBe("Retry recognition");
    expect(copyButton.disabled).toBe(true);
    retryButton.click();
    retryButton.click();
    await vi.waitFor(() => expect(f.services.ocrImage).toHaveBeenCalledTimes(2));
    expect(status(f)).toBe("loading");
    expect(copyButton.disabled).toBe(true);
    retry.resolve("recovered text");
    await vi.waitFor(() => expect(status(f)).toBe("done"));
    copyButton.click();
    await vi.waitFor(() => expect(f.contentEl.querySelector(".preview-ocr-feedback").textContent).toBe("Copy failed"));
    expect(copyButton.disabled).toBe(false);
    expect(f.contentEl.querySelector(".preview-ocr-result pre").textContent).toBe("recovered text");
  });

  it("respects disabled automatic OCR but permits one explicit recognition", async () => {
    const f = fixture({ getConfig: vi.fn().mockResolvedValue({ ocr_enabled: false }) });
    await f.renderers.renderImage(clip);
    expect(status(f)).toBe("disabled");
    expect(f.services.ocrAvailable).not.toHaveBeenCalled();
    expect(f.services.ocrImage).not.toHaveBeenCalled();
    const retryButton = f.contentEl.querySelector(".preview-ocr-retry");
    expect(retryButton.textContent).toBe("Recognize text");
    retryButton.click();
    await vi.waitFor(() => expect(status(f)).toBe("done"));
    expect(f.services.ocrImage).toHaveBeenCalledExactlyOnceWith(clip.id);
  });

  it("opens translation in the OCR section without requesting network or leaving a scan control", async () => {
    const f = fixture();
    await f.renderers.renderImage(clip);
    expect(f.contentEl.querySelector(".preview-code-scan")).toBeNull();
    f.contentEl.querySelector(".preview-ocr-translate").click();
    expect(f.imageTranslation.getSnapshot().visible).toBe(true);
    expect(f.contentEl.querySelector(".preview-ocr-source").hidden).toBe(true);
    expect(f.contentEl.querySelector(".preview-ocr-translation").hidden).toBe(false);
    f.contentEl.querySelector(".preview-ocr-back").click();
    expect(f.imageTranslation.getSnapshot().visible).toBe(false);
    expect(f.contentEl.querySelector(".preview-ocr-source").hidden).toBe(false);
    expect(f.services.detectImageCodes).not.toHaveBeenCalled();
    expect(f.services.copyText).not.toHaveBeenCalled();
    expect(f.services.openImageViewer).not.toHaveBeenCalled();
  });

  it("prevents detached actions and stale OCR completion from causing side effects", async () => {
    const ocr = deferred();
    const f = fixture({ ocrImage: vi.fn().mockReturnValue(ocr.promise) });
    await f.renderers.renderImage(clip);
    await vi.waitFor(() => expect(f.services.ocrImage).toHaveBeenCalledTimes(1));
    f.expire();
    f.contentEl.querySelector(".preview-ocr-translate").click();
    f.contentEl.querySelector(".preview-image-card button").click();
    ocr.resolve("stale text");
    await ocr.promise;
    expect(status(f)).toBe("loading");
    expect(f.imageTranslation.getSnapshot().visible).toBe(false);
    expect(f.services.openImageViewer).not.toHaveBeenCalled();
    expect(f.services.detectImageCodes).not.toHaveBeenCalled();
    expect(f.services.copyText).not.toHaveBeenCalled();
  });

  it("reports viewer failure and allows retry without discarding recognized text", async () => {
    const f = fixture({ openImageViewer: vi.fn().mockRejectedValueOnce(new Error("window failed")).mockResolvedValueOnce(undefined) });
    await f.renderers.renderImage(clip);
    await vi.waitFor(() => expect(status(f)).toBe("done"));
    const pin = f.contentEl.querySelector(".preview-image-card button");
    pin.click();
    await vi.waitFor(() => expect(pin.disabled).toBe(false));
    expect(f.contentEl.querySelector(".preview-image-card .preview-image-status").textContent).toBe("Could not open the image viewer. Please try again.");
    pin.click();
    await vi.waitFor(() => expect(f.services.openImageViewer).toHaveBeenCalledTimes(2));
    expect(f.contentEl.querySelector(".preview-ocr-result pre").textContent).toBe("<script>recognized text</script>");
  });
});

it("opens the OCR translation at the sidebar scroll origin without scrolling its containing page", () => {
  const panel = document.createElement("div");
  const scroll = document.createElement("div");
  scroll.className = "preview-scroll";
  const area = document.createElement("section"); area.className = "preview-ocr-result";
  const back = document.createElement("button"); back.className = "preview-ocr-back";
  const slot = document.createElement("div");
  area.append(back, slot); scroll.append(area); panel.append(scroll); document.body.append(panel);
  const view = createImageTranslationView();
  view.bind({ clipId: 1, container: slot, isCurrent: () => true, onVisibilityChange: vi.fn() });
  scroll.scrollTop = 200;
  scroll.getBoundingClientRect = () => ({ top: 80 });
  area.getBoundingClientRect = () => ({ top: 700 });
  scroll.scrollTo = vi.fn();
  const pageScroll = vi.spyOn(window, "scrollTo");
  revealImageTranslation(panel, view);
  expect(view.getSnapshot().visible).toBe(true);
  expect(document.activeElement).toBe(back);
  expect(scroll.scrollTo).toHaveBeenCalledExactlyOnceWith({ top: 820, behavior: "smooth" });
  expect(pageScroll).not.toHaveBeenCalled();
  panel.remove(); view.clear(); pageScroll.mockRestore();
});
