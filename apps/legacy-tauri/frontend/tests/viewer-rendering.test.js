import { afterEach, expect, it, vi } from "vitest";
import { drawScene } from "../react/annotation/canvasRenderer";
import { DEFAULT_IMAGE_ADJUSTMENTS } from "../react/annotation/imageAdjustments";

afterEach(() => vi.restoreAllMocks());
it("keeps mosaic reads within 1Mi pixels and preserves average color and partial edge cells", () => {
  const source = document.createElement("img");
  Object.defineProperties(source, { naturalWidth: { value: 4096 }, naturalHeight: { value: 513 } });
  const target = document.createElement("canvas"), reads = [], outputs = [];
  const main = Object.fromEntries(["setTransform", "save", "restore", "translate", "clearRect", "drawImage", "beginPath", "rect", "clip"].map(name => [name, vi.fn()]));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(function () {
    if (this === target) return main;
    let startY = 0;
    return { clearRect() {}, drawImage(_image, _x, y) { startY = y; },
      getImageData(_x, _y, width, height) {
        reads.push(width * height); const data = new Uint8ClampedArray(width * height * 4);
        for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) data.set([x % 256, (startY + y) % 256, 37, 128], (y * width + x) * 4);
        return { data };
      }, createImageData(width, height) { return { width, height, data: new Uint8ClampedArray(width * height * 4) }; }, putImageData(value) { outputs.push(value); } };
  });
  drawScene(target, source, { width: 800, height: 600, scale: 8, fitScale: 1, zoom: 8, pixelRatio: 2, origin: { x: -500, y: 20 } },
    [{ id: "large", type: "mosaic", rect: { x: 0, y: 0, width: 4096, height: 513 }, effect: { blurRadius: 8, mosaicCell: 256, spotlightDim: .55, magnifierZoom: 2 } }], null, DEFAULT_IMAGE_ADJUSTMENTS, null);
  expect(target.width).toBe(1600); expect(target.height).toBe(1200);
  expect(main.translate).toHaveBeenCalledWith(-500, 20);
  expect(Math.max(...reads)).toBeLessThanOrEqual(1_048_576);
  expect(outputs[0].width).toBe(16); expect(outputs[0].height).toBe(3);
  expect([...outputs[0].data.slice(0, 4)]).toEqual([128, 128, 37, 128]);
  expect([...outputs[0].data.slice(-4)]).toEqual([128, 0, 37, 128]);
});
