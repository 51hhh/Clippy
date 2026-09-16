import { beforeEach, describe, expect, it } from "vitest";
import { frameHeight, frameWidth, rgbaToFrameCanvas } from "../react/annotation/frameImage.ts";
import {
  isExportSelection,
  pngBase64ToBytes,
  stripPngDataUrl,
} from "../react/annotation/pngPipeline.ts";

describe("annotation PNG pipeline", () => {
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

describe("frame image source", () => {
  beforeEach(() => {
    // jsdom 没有 canvas 后端：只要 putImageData 不抛，尺寸校验与元素类型就能验。
    HTMLCanvasElement.prototype.getContext = () => ({ putImageData: () => {} });
    globalThis.ImageData = class {
      constructor(data, width, height) {
        Object.assign(this, { data, width, height });
      }
    };
  });

  it("measures both an <img> and an offscreen canvas as the base image", () => {
    // jsdom 不会真的加载图片，naturalWidth 又是只读的，只能直接定义。
    const image = document.createElement("img");
    Object.defineProperty(image, "naturalWidth", { value: 320 });
    Object.defineProperty(image, "naturalHeight", { value: 200 });
    expect([frameWidth(image), frameHeight(image)]).toEqual([320, 200]);

    const canvas = rgbaToFrameCanvas(new Uint8ClampedArray(4 * 4 * 2), 4, 2);
    expect([frameWidth(canvas), frameHeight(canvas)]).toEqual([4, 2]);
  });

  /** 尺寸不匹配意味着后端契约破了，宁可报错也不能把错位的像素当底图画出来。 */
  it("rejects a frame buffer that does not match the declared size", () => {
    expect(() => rgbaToFrameCanvas(new Uint8ClampedArray(10), 4, 2)).toThrow(/size mismatch/);
    expect(() => rgbaToFrameCanvas(new Uint8ClampedArray(0), 0, 0)).toThrow(/size mismatch/);
  });
});
