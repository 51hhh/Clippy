import { describe, expect, it } from "vitest";
import {
  MAX_ZOOM,
  MIN_ZOOM,
  buildViewport,
  clampZoom,
  zoomFromWheel,
} from "../react/capture/captureViewport.ts";
import {
  isExportSelection,
  pngBase64ToBytes,
  stripPngDataUrl,
} from "../react/capture/pngPipeline.ts";
import { createLatestCaptureLoader } from "../react/capture/pendingCaptureLoader.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("capture editor viewport", () => {
  it("fits the source image inside the padded stage", () => {
    expect(buildViewport(
      { clientWidth: 1000, clientHeight: 800 },
      { naturalWidth: 2000, naturalHeight: 1000 },
      1,
    )).toEqual({
      width: 976,
      height: 488,
      fitScale: 0.488,
      zoom: 1,
      scale: 0.488,
    });
  });

  it("keeps the minimum stage size and clamps zoom", () => {
    expect(buildViewport(
      { clientWidth: 100, clientHeight: 100 },
      { naturalWidth: 640, naturalHeight: 480 },
      99,
    )).toEqual({
      width: 1920,
      height: 1440,
      fitScale: 0.5,
      zoom: MAX_ZOOM,
      scale: 3,
    });
    expect(clampZoom(0.01)).toBe(MIN_ZOOM);
    expect(clampZoom(100)).toBe(MAX_ZOOM);
  });

  it("applies wheel zoom with the same exponential curve", () => {
    expect(zoomFromWheel(1, -100)).toBeCloseTo(Math.exp(0.2));
    expect(zoomFromWheel(MAX_ZOOM, -100)).toBe(MAX_ZOOM);
  });
});

describe("capture editor PNG pipeline", () => {
  it("requires a meaningful two-dimensional export selection", () => {
    expect(isExportSelection(null)).toBe(false);
    expect(isExportSelection({ x: 0, y: 0, width: 2, height: 20 })).toBe(false);
    expect(isExportSelection({ x: 0, y: 0, width: 3, height: 3 })).toBe(true);
  });

  it("decodes PNG base64 bytes and strips only PNG data URL prefixes", () => {
    expect(Array.from(pngBase64ToBytes("AAEC/w=="))).toEqual([0, 1, 2, 255]);
    expect(stripPngDataUrl("data:image/png;base64,AAEC/w==")).toBe("AAEC/w==");
    expect(stripPngDataUrl("data:image/jpeg;base64,AAEC/w==")).toBe("data:image/jpeg;base64,AAEC/w==");
  });
});

describe("pending capture request ordering", () => {
  it("applies only the newest response when an older request resolves later", async () => {
    const first = deferred();
    const second = deferred();
    const requests = [first, second];
    const loader = createLatestCaptureLoader(() => requests.shift().promise);

    const firstResult = loader.load();
    const secondResult = loader.load();
    second.resolve({ id: "new" });
    first.resolve({ id: "old" });

    await expect(secondResult).resolves.toEqual({ applied: true, value: { id: "new" } });
    await expect(firstResult).resolves.toEqual({ applied: false });
  });

  it("does not let an older failure replace a newer successful response", async () => {
    const first = deferred();
    const second = deferred();
    const requests = [first, second];
    const loader = createLatestCaptureLoader(() => requests.shift().promise);

    const firstResult = loader.load();
    const secondResult = loader.load();
    second.resolve({ id: "new" });
    first.reject(new Error("stale failure"));

    await expect(secondResult).resolves.toEqual({ applied: true, value: { id: "new" } });
    await expect(firstResult).resolves.toEqual({ applied: false });
  });

  it("invalidates both success and failure from an unmounted consumer", async () => {
    const request = deferred();
    const loader = createLatestCaptureLoader(() => request.promise);
    const result = loader.load();
    loader.invalidate();
    request.resolve({ id: "after-unmount" });

    await expect(result).resolves.toEqual({ applied: false });
  });
});
