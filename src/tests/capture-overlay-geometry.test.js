import { describe, expect, it } from "vitest";
import {
  clampRect,
  committedSelection,
  hitHandle,
  hoverCandidate,
  moveRect,
  resizeRect,
  windowAt,
} from "../react/capture-overlay/geometry.ts";

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

  it("keeps hovering windows outside an existing selection", () => {
    const windows = [{ x: 60, y: 10, width: 30, height: 30, title: "other" }];
    const selection = { x: 0, y: 0, width: 40, height: 40 };

    // 选区内部让位给移动/缩放，外面还能继续速选别的窗口
    expect(hoverCandidate(windows, { x: 20, y: 20 }, selection)).toBeNull();
    expect(hoverCandidate(windows, { x: 70, y: 20 }, selection)?.title).toBe("other");
    expect(hoverCandidate(windows, { x: 70, y: 20 }, null)?.title).toBe("other");
    expect(hoverCandidate(windows, { x: 95, y: 70 }, null)).toBeNull();
  });

  it("commits a dragged rect, a clicked window, or nothing at all", () => {
    const window = { x: 20, y: 20, width: 20, height: 20, title: "small" };

    // 拖出来的矩形原样落地
    expect(committedSelection({ x: 10, y: 10 }, { x: 70, y: 50 }, null, bounds))
      .toEqual({ x: 10, y: 10, width: 60, height: 40 });
    // 拖到屏幕外也留在边界内（与拖动过程中看到的框一致）
    expect(committedSelection({ x: 10, y: 10 }, { x: 130, y: 50 }, null, bounds))
      .toEqual({ x: 0, y: 10, width: 100, height: 40 });
    // 几乎没动就是点击：用悬停窗口速选
    expect(committedSelection({ x: 25, y: 25 }, { x: 27, y: 25 }, window, bounds))
      .toEqual({ x: 20, y: 20, width: 20, height: 20 });
    // 点在空地上不留选区
    expect(committedSelection({ x: 25, y: 25 }, { x: 27, y: 25 }, null, bounds)).toBeNull();
    // 拖出一条线没有面积，同样作废
    expect(committedSelection({ x: 10, y: 10 }, { x: 60, y: 11 }, null, bounds)).toBeNull();
  });
});
