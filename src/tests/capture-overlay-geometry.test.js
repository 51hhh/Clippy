import { describe, expect, it } from "vitest";
import { clampRect, hitHandle, moveRect, resizeRect, windowAt } from "../react/capture-overlay/geometry.ts";

const bounds = { x: 0, y: 0, width: 100, height: 80 };

describe("capture overlay geometry", () => {
  it("clamps moved selections inside monitor bounds", () => {
    expect(moveRect({ x: 70, y: 60, width: 25, height: 15 }, { x: 20, y: 20 }, bounds))
      .toEqual({ x: 75, y: 65, width: 25, height: 15 });
  });

  it("resizes from every edge without leaving bounds", () => {
    expect(resizeRect({ x: 20, y: 20, width: 40, height: 30 }, "nw", { x: -30, y: -30 }, bounds))
      .toEqual({ x: 0, y: 0, width: 60, height: 50 });
    expect(resizeRect({ x: 20, y: 20, width: 40, height: 30 }, "se", { x: 60, y: 60 }, bounds))
      .toEqual({ x: 20, y: 20, width: 80, height: 60 });
  });

  it("chooses the smallest pre-sorted smart window and detects handles", () => {
    const windows = [
      { x: 20, y: 20, width: 20, height: 20, title: "small" },
      { x: 0, y: 0, width: 100, height: 80, title: "desktop" },
    ];
    expect(windowAt(windows, { x: 25, y: 25 })?.title).toBe("small");
    expect(hitHandle(clampRect(windows[0], bounds), { x: 20, y: 20 })).toBe("nw");
  });
});
