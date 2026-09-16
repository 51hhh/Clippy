import { beforeEach, expect, it, vi } from "vitest";
const { invoke, convertFileSrc, currentWindow } = vi.hoisted(() => ({ invoke: vi.fn(), convertFileSrc: vi.fn((path, protocol) => `${protocol}://localhost/${encodeURIComponent(path)}`), currentWindow: { onResized: vi.fn(), onFocusChanged: vi.fn() } }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => currentWindow }));
import { openImageViewer, getViewerImageUrl, getViewerSettings, recognizeViewer, sampleViewerColor, copyViewerText, saveViewerImage,
  getViewerFullscreen, setViewerFullscreen, minimizeImageViewer, startViewerDrag, onViewerWindowChanged } from "../js/api";
import { viewerApi } from "../react/viewer/api";

const request = { sessionId: "session-A", snapshotId: "snapshot-A", requestId: 1 };
const payload = { handle: { sessionId: "session-A", snapshotId: "snapshot-A" }, label: "image-viewer-A", source: { width: 100, height: 100, contentHash: "a".repeat(64), byteLength: 123, mediaType: "image/png", sensitive: false }, initialProject: null, limits: { canEdit: true, canScan: true, reason: null } };
beforeEach(() => vi.clearAllMocks());
it("keeps window control authority in own-handle commands and validates the returned state", async () => {
  invoke.mockResolvedValueOnce(true); expect(await getViewerFullscreen(payload.handle)).toBe(true);
  invoke.mockResolvedValue(null);
  await setViewerFullscreen(payload.handle, false); await minimizeImageViewer(payload.handle); await startViewerDrag(payload.handle);
  expect(invoke.mock.calls).toEqual([
    ["get_viewer_fullscreen", { handle: payload.handle }], ["set_viewer_fullscreen", { handle: payload.handle, fullscreen: false }],
    ["minimize_image_viewer", { handle: payload.handle }], ["start_viewer_drag", { handle: payload.handle }],
  ]);
  await expect(getViewerFullscreen(payload.handle)).rejects.toThrow("invalid_window_state");
});
it("refreshes only this window on resize/focus and cleans up partially failed subscriptions", async () => {
  const disposeResize = vi.fn(), disposeFocus = vi.fn(), changed = vi.fn();
  currentWindow.onResized.mockResolvedValue(disposeResize); currentWindow.onFocusChanged.mockResolvedValue(disposeFocus);
  const dispose = await onViewerWindowChanged(changed);
  currentWindow.onResized.mock.calls[0][0]();
  currentWindow.onFocusChanged.mock.calls[0][0]({ payload: false });
  currentWindow.onFocusChanged.mock.calls[0][0]({ payload: true });
  expect(changed).toHaveBeenCalledTimes(2); dispose();
  expect(disposeResize).toHaveBeenCalledTimes(1); expect(disposeFocus).toHaveBeenCalledTimes(1);
  currentWindow.onFocusChanged.mockRejectedValueOnce(new Error("listener failed"));
  await expect(onViewerWindowChanged(changed)).rejects.toThrow("listener failed");
  expect(disposeResize).toHaveBeenCalledTimes(2);
});
it("loads only the viewer settings subset through production services", async () => {
  const settings = { language: "en", theme: "dark", translation_source_language: "auto", translation_target_language: "en", translation_services: [{ provider: "libretranslate", enabled: true, endpoint: "https://example.test" }] };
  invoke.mockResolvedValueOnce(settings);
  expect(viewerApi.config).toBe(getViewerSettings);
  expect(await viewerApi.config()).toEqual(settings);
  expect(invoke.mock.calls).toEqual([["get_viewer_settings"]]);
});
it("opens a bounded image payload and uses caller/snapshot bound immutable protocol URLs", async () => {
  invoke.mockResolvedValueOnce(payload); expect(await openImageViewer(3)).toEqual(payload);
  expect(invoke).toHaveBeenCalledWith("open_image_viewer", { id: 3 });
  expect(getViewerImageUrl(payload)).toBe("viewer-frame://localhost/image-viewer-A/snapshot-A");
  expect(convertFileSrc).toHaveBeenCalledWith("image-viewer-A", "viewer-frame");
  invoke.mockResolvedValueOnce({ ...payload, source: { ...payload.source, width: 16384, height: 16384 } });
  await expect(openImageViewer(3)).rejects.toThrow("invalid_payload");
});
it("rejects a late reply identity before exposing tool content", async () => {
  invoke.mockResolvedValueOnce({ ...request, snapshotId: "other", value: {} });
  await expect(recognizeViewer(request)).rejects.toThrow("stale_request");
});
it("keeps rejected raw OCR separate from final accepted copy text", async () => {
  const lines = [
    { id: 0, text: "rejected", accepted: false, confidence: .1 },
    { id: 1, text: "accepted", accepted: true, confidence: .9 },
  ].map((line, index) => ({ ...line, quad: [[0, index * 20], [100, index * 20], [100, index * 20 + 10], [0, index * 20 + 10]], charConfidences: Array.from(line.text, () => line.confidence), readingOrder: index, paragraphId: 0 }));
  const value = { width: 100, height: 100, text: "accepted", lines, paragraphs: [{ id: 0, lineIds: [0, 1], readingOrder: 0 }], pipeline: { id: "test", engine: "ppocrv6+edgegnn", featureSchema: "clippy-edge-features-v1", layoutExecuted: true, layoutReason: null }, fallbackReason: null };
  invoke.mockResolvedValueOnce({ ...request, value }); const response = await recognizeViewer(request);
  expect(response.value.text).toBe("accepted"); expect(response.value.lines[0].accepted).toBe(false);
  invoke.mockResolvedValueOnce({ ...request, value: null }); await copyViewerText(request, "ocr", 0);
  expect(invoke).toHaveBeenLastCalledWith("copy_viewer_text", { request, source: "ocr", index: 0 });
});
it("rejects unsafe sample values and sends only canonical document data for outputs", async () => {
  invoke.mockResolvedValueOnce({ ...request, value: { x: 1, y: 2, rgba: [1, 2, 3, 300], hex: "#010203", rgb: "rgb(1, 2, 3)" } });
  await expect(sampleViewerColor(request, 1, 2)).rejects.toThrow("invalid_color");
  const document = { rendererVersion: 2, sourceWidth: 100, sourceHeight: 100, annotations: [], adjustments: {} };
  invoke.mockResolvedValueOnce({ ...request, value: { path: "/image.png", clipboardWritten: false } }); await saveViewerImage(request, "flat", document);
  expect(invoke).toHaveBeenLastCalledWith("save_viewer_image", { request, mode: "flat", document });
});
it("accepts native RGB HEX with separate alpha and rejects inconsistent color text", async () => {
  const value = { x: 1, y: 2, rgba: [10, 20, 30, 40], hex: "#0A141E", rgb: "rgb(10, 20, 30)" };
  invoke.mockResolvedValueOnce({ ...request, value });
  expect((await sampleViewerColor(request, 1, 2)).value).toEqual(value);
  for (const hex of ["#0A141E28", "#FFFFFF"]) {
    invoke.mockResolvedValueOnce({ ...request, value: { ...value, hex } });
    await expect(sampleViewerColor(request, 1, 2)).rejects.toThrow("invalid_color");
  }
});
