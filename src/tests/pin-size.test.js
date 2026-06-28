import { describe, expect, it } from "vitest";
import { fitPinImageSize, resolveTempPinBaseSize } from "../js/pin-size";

describe("pin image size", () => {
  it("caps large temporary screenshot pins", () => {
    expect(fitPinImageSize(3840, 2160)).toEqual({ width: 900, height: 506 });
  });

  it("uses backend-provided temporary pin dimensions", () => {
    expect(resolveTempPinBaseSize(3840, 2160, "900", "506")).toEqual({
      width: 900,
      height: 506,
    });
  });

  it("falls back to capped dimensions when query values are invalid", () => {
    expect(resolveTempPinBaseSize(1000, 1000, "bad", "0")).toEqual({
      width: 700,
      height: 700,
    });
  });
});
