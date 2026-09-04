import { afterEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  getClipDetail: vi.fn(),
  setPreviewVisible: vi.fn(() => Promise.resolve()),
  getClipImage: vi.fn(),
  ocrAvailable: vi.fn(),
  ocrImage: vi.fn(),
  getConfig: vi.fn(),
  fetchUrlMeta: vi.fn(),
  copyText: vi.fn(),
}));

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

vi.mock("../js/api.ts", () => api);
vi.mock("../i18n/i18n.js", () => ({ t: (key) => key }));

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  vi.resetModules();
  document.body.replaceChildren();
});

function mountPreviewDom() {
  document.body.innerHTML = `
    <section id="preview-panel" class="preview-panel hidden">
      <span id="preview-type-badge"></span>
      <span id="preview-meta"></span>
      <div id="preview-content"></div>
    </section>`;
}

describe("preview render generation", () => {
  it("keeps a later item when the first lazy renderer load finishes", async () => {
    vi.useFakeTimers();
    mountPreviewDom();
    const previewPanel = await import("../js/preview-panel.js");
    previewPanel.init();
    await previewPanel.toggle();

    const first = {
      id: 1,
      content_type: "text",
      text_content: "# First delayed markdown",
      byte_size: 25,
      content_hash: "first",
    };
    const second = {
      id: 2,
      content_type: "text",
      text_content: '{"current":"second"}',
      byte_size: 20,
      content_hash: "second",
    };

    previewPanel.updatePreview(first);
    await vi.advanceTimersByTimeAsync(80);
    // 此时首次动态库加载已发起但尚未由测试推进完成；切换必须使第一代失效。
    previewPanel.updatePreview(second);
    await vi.advanceTimersByTimeAsync(80);

    await vi.waitFor(() => {
      expect(document.getElementById("preview-content").textContent).toContain('"second"');
    });
    expect(document.getElementById("preview-content").textContent).not.toContain("First delayed markdown");

    // A 已在显示、B 已排队、又回到 A 时，B 的代次已使旧 A 失效；最后一轮 A
    // 必须重新渲染，不能因为 id 相同而留下被清空的半成品。
    const backToFirst = { ...first, text_content: "# First after pending B" };
    previewPanel.updatePreview(backToFirst);
    await vi.advanceTimersByTimeAsync(80);
    await vi.waitFor(() => {
      expect(document.getElementById("preview-content").textContent).toContain("First after pending B");
    });
    previewPanel.updatePreview(second);
    previewPanel.updatePreview(backToFirst);
    await vi.advanceTimersByTimeAsync(80);
    await vi.waitFor(() => {
      expect(document.getElementById("preview-content").textContent).toContain("First after pending B");
    });

    // 同一条目偶发的重复焦点通知不能取消已经在途的图片渲染。
    const image = deferred();
    api.getClipImage.mockReturnValueOnce(image.promise);
    api.getConfig.mockResolvedValueOnce({ ocr_enabled: false });
    const third = { id: 3, content_type: "image", byte_size: 1024 };
    previewPanel.updatePreview(third);
    await vi.advanceTimersByTimeAsync(80);
    previewPanel.updatePreview(third);
    image.resolve("image-bytes");
    await vi.waitFor(() => {
      expect(document.querySelector("#preview-content img")).not.toBeNull();
    });
    expect(api.getClipImage).toHaveBeenCalledTimes(1);
  });

  it("does not let a rejected stale HTML detail fall back over the next item", async () => {
    vi.useFakeTimers();
    mountPreviewDom();
    const detail = deferred();
    api.getClipDetail.mockReturnValueOnce(detail.promise);
    const previewPanel = await import("../js/preview-panel.js");
    previewPanel.init();
    await previewPanel.toggle();

    previewPanel.updatePreview({
      id: 11,
      content_type: "html",
      text_content: "alpha",
      byte_size: 5,
      content_hash: "html-a",
    });
    await vi.advanceTimersByTimeAsync(80);
    await vi.waitFor(() => expect(api.getClipDetail).toHaveBeenCalledWith(11));

    previewPanel.updatePreview({
      id: 12,
      content_type: "text",
      text_content: "B",
      byte_size: 1,
      content_hash: "text-b",
    });
    await vi.advanceTimersByTimeAsync(80);
    detail.reject(new Error("stale detail failed"));
    await Promise.resolve();

    expect(document.getElementById("preview-content").textContent).toBe("B");
  });

  it("fully rerenders A when a queued B switch is cancelled by returning to A", async () => {
    vi.useFakeTimers();
    mountPreviewDom();
    const firstImage = deferred();
    api.getClipImage
      .mockReturnValueOnce(firstImage.promise)
      .mockResolvedValueOnce("current-a");
    api.getConfig.mockResolvedValue({ ocr_enabled: false });
    const previewPanel = await import("../js/preview-panel.js");
    previewPanel.init();
    await previewPanel.toggle();

    const imageA = { id: 21, content_type: "image", byte_size: 1024 };
    previewPanel.updatePreview(imageA);
    await vi.advanceTimersByTimeAsync(80);
    previewPanel.updatePreview({ id: 22, content_type: "text", text_content: "B", byte_size: 1 });
    previewPanel.updatePreview(imageA);
    await vi.advanceTimersByTimeAsync(80);
    firstImage.resolve("stale-a");
    await Promise.resolve();

    expect(api.getClipImage).toHaveBeenCalledTimes(2);
    expect(document.querySelectorAll("#preview-content img")).toHaveLength(1);
    expect(document.getElementById("preview-content").textContent).not.toContain("B");
  });
});
