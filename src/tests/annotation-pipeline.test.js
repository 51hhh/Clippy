import { describe, expect, it } from "vitest";
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
