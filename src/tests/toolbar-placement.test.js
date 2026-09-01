import { describe, expect, it } from "vitest";
import {
  clampToolbarPosition,
  horizontalToolbarPlacement,
  verticalToolbarPlacement,
} from "../react/shared/toolbarPlacement.ts";

const VIEWPORT = { width: 1000, height: 800 };
const HORIZONTAL = { width: 400, height: 80 };
const VERTICAL = { width: 40, height: 250 };

describe("horizontal toolbar placement", () => {
  it("prefers below the anchor", () => {
    const spot = horizontalToolbarPlacement(
      { x: 100, y: 100, width: 300, height: 200 },
      HORIZONTAL,
      VIEWPORT,
    );
    expect(spot.placement).toBe("below");
    expect(spot.top).toBe(308);
    // 与锚点右边缘对齐：400 宽的工具条右边贴在 x=400 上
    expect(spot.left).toBe(0 + 8);
  });

  it("flips above when there is no room below", () => {
    const spot = horizontalToolbarPlacement(
      { x: 100, y: 500, width: 300, height: 280 },
      HORIZONTAL,
      VIEWPORT,
    );
    expect(spot.placement).toBe("above");
    expect(spot.top).toBe(500 - 80 - 8);
  });

  /** 上下都没地方（锚点顶满整屏）才进内部，而且要贴在内容底边上方。 */
  it("moves inside when neither side fits", () => {
    const spot = horizontalToolbarPlacement(
      { x: 0, y: 0, width: 1000, height: 800 },
      HORIZONTAL,
      VIEWPORT,
    );
    expect(spot.placement).toBe("inside");
    expect(spot.top).toBe(800 - 80 - 8);
    expect(spot.top + HORIZONTAL.height).toBeLessThanOrEqual(VIEWPORT.height);
  });
});

describe("vertical toolbar placement", () => {
  /** 贴图工具条的默认位置：内容区右侧。 */
  it("prefers the right side of the anchor", () => {
    const spot = verticalToolbarPlacement(
      { x: 100, y: 100, width: 300, height: 200 },
      VERTICAL,
      VIEWPORT,
    );
    expect(spot).toEqual({ left: 408, top: 100, placement: "right" });
  });

  /**
   * 这条就是那个 bug：贴图贴在屏幕右边缘时，工具条以前钉死在窗口右上角，
   * 于是整条在屏幕外面，一个按钮都点不到。现在必须翻到左侧。
   */
  it("flips to the left when the anchor is against the right edge", () => {
    const spot = verticalToolbarPlacement(
      { x: 600, y: 100, width: 392, height: 200 },
      VERTICAL,
      VIEWPORT,
    );
    expect(spot.placement).toBe("left");
    expect(spot.left).toBe(600 - 40 - 8);
  });

  it("moves inside when neither side fits", () => {
    const spot = verticalToolbarPlacement(
      { x: 0, y: 0, width: 1000, height: 600 },
      VERTICAL,
      VIEWPORT,
    );
    expect(spot.placement).toBe("inside");
    expect(spot.left + VERTICAL.width).toBeLessThanOrEqual(VIEWPORT.width);
  });

  /** 锚点高过视口时纵向要钳住，否则工具条上半截跑到屏幕外。 */
  it("clamps the vertical position into the viewport", () => {
    const spot = verticalToolbarPlacement(
      { x: 100, y: 700, width: 200, height: 400 },
      VERTICAL,
      VIEWPORT,
    );
    expect(spot.top).toBe(800 - 250 - 8);
  });
});

describe("manual toolbar position", () => {
  /** 拖到屏幕外就再也拖不回来了，所以钳制不能省。 */
  it("keeps a dragged toolbar reachable", () => {
    expect(clampToolbarPosition({ left: -200, top: -80 }, VERTICAL, VIEWPORT)).toEqual({
      left: 8,
      top: 8,
    });
    expect(clampToolbarPosition({ left: 5000, top: 5000 }, VERTICAL, VIEWPORT)).toEqual({
      left: 1000 - 40 - 8,
      top: 800 - 250 - 8,
    });
    // 视口里的位置原样保留
    expect(clampToolbarPosition({ left: 120, top: 200 }, VERTICAL, VIEWPORT)).toEqual({
      left: 120,
      top: 200,
    });
  });

  /** 工具条比视口还大（极小的贴图窗口）时退化成贴边，不能算出负数。 */
  it("degrades to the edge when the toolbar is larger than the viewport", () => {
    expect(clampToolbarPosition({ left: 50, top: 50 }, { width: 400, height: 400 }, {
      width: 200,
      height: 200,
    })).toEqual({ left: 8, top: 8 });
  });
});
