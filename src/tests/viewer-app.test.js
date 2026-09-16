import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { ViewerDocument } from "../react/viewer/App";
import { init } from "../i18n/i18n.js";

const payload = (id = "A", sensitive = false) => ({ handle: { sessionId: `session-${id}`, snapshotId: `snapshot-${id}` }, label: `image-viewer-${id}`,
  source: { width: 1200, height: 800, contentHash: "a".repeat(64), byteLength: 1000, mediaType: "image/png", clipId: 1, sensitive },
  initialProject: null, limits: { canEdit: true, canScan: true, reason: null } });
const config = { translation_target_language: "en", translation_services: [{ provider: "libretranslate", enabled: true, endpoint: "https://example.test/" }] };
const result = (request, value) => ({ ...request, value });
const ocr = text => ({ width: 1200, height: 800, text, lines: [], paragraphs: [], pipeline: { engine: "tesseract", id: "test", featureSchema: null, layoutExecuted: false, layoutReason: null }, fallbackReason: null });
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
let root, services, nativeClose, captured, ctx, windowChanged, windowFullscreen, area;
const button = label => document.querySelector(`button[aria-label="${label}"]`) || [...document.querySelectorAll("button")].find(node => node.textContent === label);
const click = async label => { const node = typeof label === "string" ? button(label) : label; expect(node).toBeTruthy(); await act(async () => node.click()); };
const canvas = () => document.querySelector(".viewer-canvas");
async function pointer(type, x, y, node = canvas()) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0 });
  Object.defineProperty(event, "pointerId", { value: 1 });
  await act(async () => node.dispatchEvent(event));
}
async function draw() {
  await click("Drawing tools");
  await pointer("pointerdown", 400, 260); await pointer("pointermove", 440, 290); await pointer("pointerup", 440, 290);
}
async function mount(id = "A", sensitive = false) {
  await act(async () => root.render(React.createElement(ViewerDocument, { key: id, payload: payload(id, sensitive), config, services })));
}
beforeEach(async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true; init("en"); captured = new Set();
  windowFullscreen = false; area = { width: 800, height: 600 };
  HTMLElement.prototype.setPointerCapture = id => captured.add(id);
  HTMLElement.prototype.hasPointerCapture = id => captured.has(id);
  HTMLElement.prototype.releasePointerCapture = id => captured.delete(id);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("Image", class { naturalWidth = 1200; naturalHeight = 800; set src(value) { this._src = value; queueMicrotask(() => this.onload?.()); } get src() { return this._src; } });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function () {
    return this.classList?.contains("viewer-floating-toolbar") ? { left: 0, top: 0, width: 400, height: 90 } : { left: 0, top: 0, ...area };
  });
  ctx = Object.fromEntries(["setTransform", "clearRect", "drawImage", "save", "restore", "translate", "beginPath", "moveTo", "lineTo", "stroke", "setLineDash", "strokeRect", "fill", "clip", "rect"].map(name => [name, vi.fn()]));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx);
  services = {
    get: vi.fn(async () => payload()), config: vi.fn(async () => config), imageUrl: vi.fn(() => "synthetic"), ready: vi.fn(async () => {}), close: vi.fn(async () => {}),
    onCloseRequested: vi.fn(async cb => { nativeClose = cb; return vi.fn(); }),
    getFullscreen: vi.fn(async () => windowFullscreen),
    setFullscreen: vi.fn(async (_, value) => { windowFullscreen = value; }),
    minimize: vi.fn(async () => {}), startDrag: vi.fn(async () => {}),
    onWindowChanged: vi.fn(async cb => { windowChanged = cb; return vi.fn(); }),
    recognize: vi.fn(async request => result(request, ocr("recognized text"))),
    scan: vi.fn(async request => result(request, { results: [{ format: "qr_code", text: "https://example.test/", points: [] }], limited: false })),
    translate: vi.fn(async request => result(request, { request_id: request.requestId, services: [{ provider: "libretranslate", status: "ok", translated_text: "translated", target_language: "en" }] })),
    sample: vi.fn(async (request, x, y) => result(request, { x, y, rgba: [1, 2, 3, 4], hex: "#010203", rgb: "rgb(1, 2, 3)" })),
    copyImage: vi.fn(async request => result(request, null)), save: vi.fn(async request => result(request, { path: "/review.png", clipboardWritten: true, clipboardError: null })),
    pin: vi.fn(async request => result(request, "pin-viewer")), copyText: vi.fn(async request => result(request, null)),
  };
  const host = document.createElement("div"); document.body.replaceChildren(host); root = createRoot(host); await mount();
});
afterEach(async () => { await act(async () => root.unmount()); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete globalThis.IS_REACT_ACT_ENVIRONMENT; });

describe("production viewer workflows", () => {
  it("keeps unlimited pan across keyboard movement, resize and DPR changes, and fit finds the image", async () => {
    await pointer("pointerdown", 400, 300); await pointer("pointermove", 3400, -1700); await pointer("pointerup", 3400, -1700);
    expect(Number(canvas().dataset.panX)).toBe(3000); expect(Number(canvas().dataset.panY)).toBe(-2028);
    await act(async () => { for (let i = 0; i < 30; i++) canvas().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })); });
    expect(Number(canvas().dataset.panX)).toBe(4440);
    const panY = canvas().dataset.panY;
    area = { width: 600, height: 400 }; vi.stubGlobal("devicePixelRatio", 2);
    await act(async () => window.dispatchEvent(new Event("resize")));
    expect(Number(canvas().dataset.panX)).toBe(4440); expect(canvas().dataset.panY).toBe(panY);
    expect(canvas().width).toBe(1200); expect(canvas().height).toBe(800);
    await click("Text recognition"); expect(Number(canvas().dataset.panX)).toBe(4440);
    await click("Fit image"); expect(canvas().dataset.panX).toBe("0");
  });
  it("uses native fullscreen state, consumes Escape before tool/dirty close and guards modal controls", async () => {
    await draw(); await click("Text recognition");
    windowFullscreen = true; await act(async () => windowChanged());
    expect(button("Exit fullscreen")).toBeTruthy();
    await act(async () => canvas().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(services.setFullscreen).toHaveBeenLastCalledWith(payload().handle, false);
    expect(document.querySelector(".viewer-panel")).not.toBeNull(); expect(services.close).not.toHaveBeenCalled();
    await click("Close viewer"); expect(document.querySelector("[data-pin-dialog]")).not.toBeNull();
    const f11 = new KeyboardEvent("keydown", { key: "F11", bubbles: true, cancelable: true });
    await act(async () => document.querySelector("[data-pin-dialog]").dispatchEvent(f11));
    expect(f11.defaultPrevented).toBe(true);
    await click("Enter fullscreen"); await click("Minimize viewer");
    await pointer("pointerdown", 100, 20, document.querySelector(".viewer-header h1"));
    expect(services.setFullscreen).toHaveBeenCalledTimes(1); expect(services.minimize).not.toHaveBeenCalled(); expect(services.startDrag).not.toHaveBeenCalled();
    await click("Cancel"); expect(document.querySelector('[aria-label="Unsaved changes"]')).not.toBeNull();
    await click("Minimize viewer"); expect(services.minimize).toHaveBeenCalledWith(payload().handle); expect(services.close).not.toHaveBeenCalled();
  });
  it("waits for authoritative fullscreen before Escape and cannot close through a later save modal", async () => {
    const old = deferred(); services.getFullscreen.mockReturnValueOnce(old.promise);
    await act(async () => windowChanged()); windowFullscreen = true;
    await act(async () => canvas().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(services.close).not.toHaveBeenCalled();
    await act(async () => old.resolve(false));
    expect(services.setFullscreen).toHaveBeenLastCalledWith(payload().handle, false);
    expect(services.close).not.toHaveBeenCalled();
    const pending = deferred(); services.getFullscreen.mockReturnValueOnce(pending.promise);
    await act(async () => canvas().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    await click("Save image");
    await act(async () => pending.resolve(false));
    expect(document.querySelector("[data-pin-dialog]")).not.toBeNull(); expect(services.close).not.toHaveBeenCalled();
  });
  it("does not close on a failed Escape state query and refreshes after window events", async () => {
    services.getFullscreen.mockRejectedValueOnce({ code: "window_failed" });
    await act(async () => canvas().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(services.close).not.toHaveBeenCalled(); expect(document.querySelector('[role="alert"]')).not.toBeNull();
    windowFullscreen = true; await act(async () => windowChanged());
    expect(button("Exit fullscreen")).toBeTruthy(); expect(document.querySelector('[role="alert"]')).toBeNull();
    const f11 = new KeyboardEvent("keydown", { key: "F11", bubbles: true, cancelable: true });
    await act(async () => canvas().dispatchEvent(f11));
    expect(f11.defaultPrevented).toBe(true); expect(services.setFullscreen).toHaveBeenLastCalledWith(payload().handle, false);
  });
  it("serializes window actions and ignores an old state query after a fullscreen command", async () => {
    const old = deferred(), change = deferred();
    services.getFullscreen.mockReturnValueOnce(old.promise);
    await act(async () => windowChanged());
    services.setFullscreen.mockImplementationOnce(async (_, value) => { await change.promise; windowFullscreen = value; });
    await click("Enter fullscreen"); await click("Enter fullscreen"); await click("Minimize viewer");
    expect(services.setFullscreen).toHaveBeenCalledTimes(1); expect(services.minimize).not.toHaveBeenCalled();
    await act(async () => { change.resolve(); old.resolve(false); });
    expect(button("Exit fullscreen")).toBeTruthy();
  });
  it("keeps window failures visible and retryable without leaking stale session errors", async () => {
    services.setFullscreen.mockRejectedValueOnce({ code: "window_failed" });
    await click("Enter fullscreen"); expect(document.querySelector('[role="alert"]').textContent).toContain("window");
    expect(button("Enter fullscreen").disabled).toBe(false);
    await click("Enter fullscreen"); expect(button("Exit fullscreen")).toBeTruthy();
    await pointer("pointerdown", 100, 20, document.querySelector(".viewer-header h1")); expect(services.startDrag).toHaveBeenCalledTimes(1);
    await pointer("pointerdown", 100, 20, button("Close viewer")); expect(services.startDrag).toHaveBeenCalledTimes(1);
    const pending = deferred(); services.minimize.mockReturnValueOnce(pending.promise); await click("Minimize viewer");
    await mount("B"); await act(async () => pending.reject({ code: "window_failed" }));
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });
  it("retains uncertain OCR in collapsed details and copies only accepted final text", async () => {
    const lines = [
      { id: 0, text: "accepted", accepted: true, confidence: .9 },
      { id: 1, text: "uncertain raw", accepted: false, confidence: .3 },
    ].map((line, index) => ({ ...line, readingOrder: index, paragraphId: 0, charConfidences: Array.from(line.text, () => line.confidence), quad: [[0, index * 20], [100, index * 20], [100, index * 20 + 20], [0, index * 20 + 20]] }));
    const value = { ...ocr("accepted"), lines, paragraphs: [{ id: 0, lineIds: [0, 1], readingOrder: 0 }], pipeline: { ...ocr("").pipeline, engine: "ppocrv6+edgegnn" } };
    services.recognize.mockImplementationOnce(async request => result(request, value));
    await click("Text recognition"); await click("Recognize text");
    const details = document.querySelector(".viewer-ocr-details");
    expect(details.open).toBe(false); expect(details.textContent).toContain("uncertain raw"); expect(details.textContent).toContain("30%");
    expect(document.querySelector(".viewer-panel pre").textContent).toBe("accepted");
    await click(document.querySelector(".viewer-copy-text")); expect(services.copyText.mock.calls[0].slice(1)).toEqual(["ocr", 0]);
    services.recognize.mockImplementationOnce(async request => result(request, { ...value, text: "", lines: [{ ...lines[1], id: 0, readingOrder: 0 }], paragraphs: [{ id: 0, lineIds: [0], readingOrder: 0 }] }));
    await click("Recognize text"); expect(document.querySelector(".viewer-copy-text").disabled).toBe(true);
    expect(document.querySelector(".viewer-panel pre").textContent).toContain("No reliable text");
  });
  it("distinguishes fallback categories without displaying unknown process paths", async () => {
    await click("Text recognition");
    for (const [fallbackReason, expected] of [["enhanced_configuration_invalid", "configuration could not be validated"], ["enhanced_failed", "could not finish recognition"], ["/private/model-secret.onnx", "engine was unavailable"]]) {
      services.recognize.mockImplementationOnce(async request => result(request, { ...ocr("fallback"), fallbackReason }));
      await click("Recognize text"); expect(document.querySelector(".viewer-panel").textContent).toContain(expected);
      expect(document.querySelector(".viewer-panel").textContent).not.toContain("/private/");
    }
  });
  it("locks further translation after a sensitive provider error in a successful batch", async () => {
    services.translate.mockImplementationOnce(async request => result(request, { request_id: request.requestId, services: [
      { provider: "libretranslate", status: "ok", translated_text: "completed earlier", target_language: "en" },
      { provider: "deepl", status: "error", code: "sensitive_content" },
    ] }));
    await click("Translate text"); await click(document.querySelector(".viewer-panel .viewer-primary-button"));
    expect(document.querySelector(".viewer-panel .viewer-primary-button").disabled).toBe(true);
    expect(document.querySelector(".viewer-panel").textContent).toContain("completed earlier");
    await click(document.querySelector(".viewer-panel .viewer-primary-button")); expect(services.translate).toHaveBeenCalledTimes(1);
  });
  it("keeps tools explicit, uses trusted copy sources, and never turns scanned URLs into links", async () => {
    await click("Text recognition"); expect(services.recognize).not.toHaveBeenCalled();
    await click("Recognize text"); expect(document.querySelector(".viewer-panel pre").textContent).toBe("recognized text");
    await click(document.querySelector(".viewer-copy-text"));
    expect(services.copyText.mock.calls[0].slice(1)).toEqual(["ocr", 0]);
    await click("Scan image codes"); await click(document.querySelector(".viewer-panel .viewer-primary-button"));
    expect(document.querySelector(".viewer-panel a")).toBeNull();
    expect(document.querySelector(".viewer-panel pre").textContent).toBe("https://example.test/");
  });
  it("discards late results across immutable snapshot remounts", async () => {
    const pending = deferred(); services.recognize.mockReturnValueOnce(pending.promise);
    await click("Text recognition"); await click("Recognize text"); const request = services.recognize.mock.calls[0][0];
    await mount("B"); await act(async () => pending.resolve(result(request, ocr("stale A"))));
    await click("Text recognition"); expect(document.body.textContent).not.toContain("stale A");
    expect(document.querySelector("[data-snapshot-id]").dataset.snapshotId).toBe("snapshot-B");
  });
  it("uses physical 100%, inverse source sampling and viewport-sized backing stores", async () => {
    vi.stubGlobal("devicePixelRatio", 2); await mount("DPR");
    await click("100% — one source pixel per device pixel");
    expect(canvas().dataset.scale).toBe("0.5");
    expect(canvas().width).toBe(1600); expect(canvas().height).toBe(1200);
    await click("Pick a color"); await pointer("pointerdown", 400, 300);
    expect(services.sample.mock.calls[0].slice(1)).toEqual([600, 400]);
    expect(document.querySelector(".viewer-panel").textContent).toContain("#010203");
  });
  it("saves an actual annotation document, freezes busy editing and closes only after success", async () => {
    await draw(); expect(document.querySelector('[aria-label="Unsaved changes"]')).not.toBeNull();
    await act(async () => nativeClose()); expect(document.querySelector('[data-pin-dialog]')).not.toBeNull();
    const pending = deferred(); services.save.mockReturnValueOnce(pending.promise);
    await click("Save and close");
    const [request, mode, project] = services.save.mock.calls[0];
    expect(mode).toBe("editable"); expect(project.annotations.length).toBe(1);
    await pointer("pointerdown", 430, 260); await pointer("pointermove", 490, 300); await pointer("pointerup", 490, 300);
    await act(async () => nativeClose()); expect(services.save).toHaveBeenCalledTimes(1); expect(services.close).not.toHaveBeenCalled();
    await act(async () => pending.resolve(result(request, { path: "/review.png", clipboardWritten: true, clipboardError: null })));
    expect(services.close).toHaveBeenCalledTimes(1);
  });
  it("retains dirty edits after failure, restores focus on cancel, and keeps partial save success", async () => {
    await draw(); const save = button("Save image"); await act(async () => save.focus()); await click(save);
    services.save.mockRejectedValueOnce({ code: "save_failed" }); await click("Save editable PNG");
    expect(document.querySelector('[data-pin-dialog] [role="alert"]')).not.toBeNull();
    await click("Cancel"); expect(document.activeElement).toBe(save); expect(document.querySelector('[aria-label="Unsaved changes"]')).not.toBeNull();
    await click("Save image"); services.save.mockImplementationOnce(async request => result(request, { path: "/saved.png", clipboardWritten: false, clipboardError: "busy" })); await click("Save editable PNG");
    expect(document.querySelector('[aria-label="Unsaved changes"]')).toBeNull(); expect(document.body.textContent).toContain("file was saved"); expect(services.close).not.toHaveBeenCalled();
  });
  it("protects sensitive translation and does not retry uncertain Pin creation", async () => {
    await mount("sensitive", true); await click("Translate text");
    expect(document.querySelector(".viewer-panel .viewer-primary-button").disabled).toBe(true); expect(services.translate).not.toHaveBeenCalled();
    services.pin.mockRejectedValueOnce({ code: "pin_creation_uncertain" }); await click("Open as Pin");
    expect(button("Open as Pin").disabled).toBe(true); await click("Open as Pin"); expect(services.pin).toHaveBeenCalledTimes(1);
  });
  it("clears Space/pointer input on blur and keeps toolbar pointer and wheel outside the image", async () => {
    await click("Pick a color"); await click("100% — one source pixel per device pixel");
    await act(async () => { canvas().focus(); canvas().dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })); window.dispatchEvent(new FocusEvent("blur")); });
    await pointer("pointerdown", 400, 300); expect(services.sample).toHaveBeenCalledTimes(1);
    const before = canvas().dataset.scale;
    const toolbar = document.querySelector(".viewer-floating-toolbar");
    await act(async () => toolbar.dispatchEvent(new WheelEvent("wheel", { deltaY: -200, bubbles: true, cancelable: true })));
    expect(canvas().dataset.scale).toBe(before);
    await pointer("pointerdown", 400, 300, button("Text recognition")); expect(services.sample).toHaveBeenCalledTimes(1);
  });
});
