import { afterEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  getClipDetail: vi.fn(),
  setPreviewVisible: vi.fn(() => Promise.resolve()),
  getClipImage: vi.fn(),
  detectImageCodes: vi.fn(),
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

  it("does not let a completed stale explicit scan write over the next preview", async () => {
    vi.useFakeTimers();
    mountPreviewDom();
    const scan = deferred();
    api.getClipImage.mockResolvedValue("image-bytes");
    api.getConfig.mockResolvedValue({ ocr_enabled: false });
    api.detectImageCodes.mockReturnValue(scan.promise);
    const previewPanel = await import("../js/preview-panel.js");
    previewPanel.init();
    await previewPanel.toggle();

    previewPanel.updatePreview({ id: 31, content_type: "image", byte_size: 1024 });
    await vi.advanceTimersByTimeAsync(80);
    await vi.waitFor(() => {
      expect(document.querySelector(".preview-code-scan-button")).not.toBeNull();
    });
    document.querySelector(".preview-code-scan-button").click();
    expect(api.detectImageCodes).toHaveBeenCalledWith(31);

    previewPanel.updatePreview({ id: 32, content_type: "text", text_content: "current", byte_size: 1 });
    await vi.advanceTimersByTimeAsync(80);
    scan.resolve({ results: [{ format: "qr_code", text: "late scan", points: [] }], limited: false });
    await Promise.resolve();
    await Promise.resolve();

    expect(document.getElementById("preview-content").textContent).toContain("current");
    expect(document.getElementById("preview-content").textContent).not.toContain("late scan");
  });

  it("does not publish a rejected scan after the preview is hidden", async () => {
    vi.useFakeTimers();
    mountPreviewDom();
    const scan = deferred();
    api.getClipImage.mockResolvedValue("image-bytes");
    api.getConfig.mockResolvedValue({ ocr_enabled: false });
    api.detectImageCodes.mockReturnValue(scan.promise);
    const previewPanel = await import("../js/preview-panel.js");
    previewPanel.init();
    await previewPanel.toggle();

    previewPanel.updatePreview({ id: 33, content_type: "image", byte_size: 1024 });
    await vi.advanceTimersByTimeAsync(80);
    await vi.waitFor(() => {
      expect(document.querySelector(".preview-code-scan-button")).not.toBeNull();
    });
    const area = document.querySelector(".preview-code-scan");
    area.querySelector(".preview-code-scan-button").click();
    expect(area.dataset.status).toBe("loading");

    await previewPanel.hide();
    scan.reject(new Error("late scan failure"));
    await Promise.resolve();
    await Promise.resolve();

    expect(document.getElementById("preview-panel").classList.contains("hidden")).toBe(true);
    expect(area.dataset.status).toBe("loading");
    expect(area.querySelector(".preview-code-scan-status").textContent).toBe("codeScan.scanning");
  });
});

describe('预算在预览派发前生效', () => {
  it('超限 JSON/JWT/Base64 只展示带说明的原文片段，原条目不变', async () => {
    vi.useFakeTimers(); mountPreviewDom();
    const { MAX_RENDER_CHARS } = await import('../js/preview/large-text.js');
    const preview = await import('../js/preview-panel.js'); preview.init(); await preview.toggle();
    const texts = [JSON.stringify({ body: 'x'.repeat(MAX_RENDER_CHARS) }), 'eyJ.' + 'A'.repeat(MAX_RENDER_CHARS) + '.abc', btoa('a'.repeat(MAX_RENDER_CHARS))];
    for (const [index, text] of texts.entries()) {
      const entry = { id: index + 501, content_type: 'text', text_content: text, byte_size: text.length };
      preview.updatePreview(entry); await vi.advanceTimersByTimeAsync(80);
      const content = document.getElementById('preview-content');
      expect(document.getElementById('preview-type-badge').textContent).toBe('TEXT');
      expect(content.firstChild.textContent.length).toBe(MAX_RENDER_CHARS);
      expect(content.querySelector('.preview-truncated')).not.toBeNull();
      expect(content.querySelector('code, img')).toBeNull(); expect(entry.text_content).toBe(text);
    }
  });

  it('独立 HTML 详情超过预算时不解析截断标签', async () => {
    vi.useFakeTimers(); mountPreviewDom();
    const { MAX_RENDER_CHARS } = await import('../js/preview/large-text.js');
    api.getClipDetail.mockResolvedValue({ html_content: '<p>' + 'x'.repeat(MAX_RENDER_CHARS) + '</p>' });
    const preview = await import('../js/preview-panel.js'); preview.init(); await preview.toggle();
    preview.updatePreview({ id: 601, content_type: 'html', text_content: 'alpha', byte_size: MAX_RENDER_CHARS + 7 });
    await vi.advanceTimersByTimeAsync(80);
    await vi.waitFor(() => expect(document.querySelector('#preview-content .preview-truncated')).not.toBeNull());
    expect(document.getElementById('preview-type-badge').textContent).toBe('TEXT');
    expect(document.querySelector('#preview-content p')).toBeNull();
    expect(document.getElementById('preview-content').textContent.startsWith('<p>')).toBe(true);
  });
});
