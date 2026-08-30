import { describe, expect, it } from "vitest";
import {
  clampRect,
  committedSelection,
  coversBounds,
  hitHandle,
  hoverCandidate,
  moveRect,
  resizeRect,
  toolbarPlacement,
  toPixelRect,
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

  it("commits a dragged rect, a clicked window, or the whole screen", () => {
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
    // 点在空地上就是整屏（参考项目里"直接点一下即全屏"的手感）
    expect(committedSelection({ x: 25, y: 25 }, { x: 27, y: 25 }, null, bounds)).toEqual(bounds);
    // 拖出一条线没有面积，同样作废
    expect(committedSelection({ x: 10, y: 10 }, { x: 60, y: 11 }, null, bounds)).toBeNull();
  });

  it("knows when a selection already covers the whole monitor", () => {
    expect(coversBounds(bounds, bounds)).toBe(true);
    // 只差一像素也算铺满：钳位与取整会产生这种误差
    expect(coversBounds({ x: 1, y: 0, width: 99, height: 80 }, bounds)).toBe(true);
    expect(coversBounds({ x: 0, y: 0, width: 60, height: 80 }, bounds)).toBe(false);
  });

  it("converts a logical selection into frozen-frame pixels", () => {
    const frame = { x: 0, y: 0, width: 200, height: 160 };

    // 逻辑 100×80 的桌面对应 200×160 的冻结帧：缩放因子 2
    expect(toPixelRect({ x: 10, y: 20, width: 30, height: 40 }, 2, 2, frame))
      .toEqual({ x: 20, y: 40, width: 60, height: 80 });
    // 越界的选区钳进帧内，且至少留 1px，`renderExport` 不会采样到帧外
    expect(toPixelRect({ x: 90, y: 70, width: 40, height: 40 }, 2, 2, frame))
      .toEqual({ x: 180, y: 140, width: 20, height: 20 });
    expect(toPixelRect({ x: 100, y: 80, width: 10, height: 10 }, 2, 2, frame))
      .toEqual({ x: 200, y: 160, width: 1, height: 1 });
  });

  it("places the toolbar below the selection, flipping or clamping when it does not fit", () => {
    const toolbar = { width: 60, height: 20 };
    const viewport = { width: 100, height: 80 };

    // 下方放得下：贴在选区下沿；工具条比选区宽时靠左钳进视口
    expect(toolbarPlacement({ x: 10, y: 10, width: 40, height: 20 }, toolbar, viewport))
      .toEqual({ left: 8, top: 38 });
    // 选区贴着屏幕底部：翻到上方
    expect(toolbarPlacement({ x: 10, y: 40, width: 40, height: 40 }, toolbar, viewport))
      .toEqual({ left: 8, top: 12 });
    // 上下都放不下：压在视口底部，不跑到屏幕外
    const squeezed = toolbarPlacement({ x: 0, y: 0, width: 100, height: 80 }, toolbar, viewport);
    expect(squeezed.top).toBe(52);
    expect(squeezed.left + toolbar.width).toBeLessThanOrEqual(viewport.width);
  });
});
