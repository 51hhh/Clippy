import { describe, expect, it } from "vitest";
import {
  cssFilterForImageAdjustments,
  hasImageAdjustments,
  normalizeImageAdjustments,
} from "../react/annotation/imageAdjustments";

describe("annotation image adjustments", () => {
  it("clamps slider values", () => {
    expect(normalizeImageAdjustments({ brightness: 120, contrast: -140, saturation: 4.4, cornerRadius: 999 })).toEqual({
      grayscale: false,
      brightness: 100,
      contrast: -100,
      saturation: 4,
      cornerRadius: 120,
    });
  });

  it("detects no-op adjustments", () => {
    expect(hasImageAdjustments(normalizeImageAdjustments())).toBe(false);
    expect(hasImageAdjustments(normalizeImageAdjustments({ grayscale: true }))).toBe(true);
  });

  it("builds a CSS filter string", () => {
    expect(
      cssFilterForImageAdjustments({
        grayscale: true,
        brightness: 10,
        contrast: -20,
        saturation: 30,
        cornerRadius: 0,
      }),
    ).toBe("grayscale(1) brightness(110%) contrast(80%) saturate(130%)");
  });
});
