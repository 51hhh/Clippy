import { describe, expect, it } from "vitest";
import {
  clampToolbarPosition,
  fullWindowBounds,
  horizontalToolbarPlacement,
  verticalToolbarPlacement,
} from "../react/shared/toolbarPlacement.ts";

const WINDOW = fullWindowBounds({ width: 1000, height: 800 });
const HORIZONTAL = { width: 400, height: 80 };
const VERTICAL = { width: 40, height: 250 };

describe("horizontal toolbar placement", () => {
  it("prefers below the anchor", () => {
    const spot = horizontalToolbarPlacement(
      { x: 100, y: 100, width: 300, height: 200 },
      HORIZONTAL,
      WINDOW,
    );
    expect(spot.placement).toBe("below");
    expect(spot.top).toBe(308);
    expect(spot.left).toBe(8);
  });

  it("flips above when there is no room below", () => {
    const spot = horizontalToolbarPlacement(
      { x: 100, y: 500, width: 300, height: 280 },
      HORIZONTAL,
      WINDOW,
    );
    expect(spot.placement).toBe("above");
    expect(spot.top).toBe(500 - 80 - 8);
  });

  it("moves inside when neither side fits", () => {
    const spot = horizontalToolbarPlacement(
      { x: 0, y: 0, width: 1000, height: 800 },
      HORIZONTAL,
      WINDOW,
    );
    expect(spot.placement).toBe("inside");
    expect(spot.top).toBe(800 - 80 - 8);
    expect(spot.top + HORIZONTAL.height).toBeLessThanOrEqual(WINDOW.height);
  });
});

describe("vertical toolbar placement", () => {
  it("prefers the right side of the anchor", () => {
    const spot = verticalToolbarPlacement(
      { x: 100, y: 100, width: 300, height: 200 },
      VERTICAL,
      WINDOW,
    );
    expect(spot).toEqual({ left: 408, top: 100, placement: "right" });
  });

  it("flips to the left when the anchor is against the right edge", () => {
    const spot = verticalToolbarPlacement(
      { x: 600, y: 100, width: 392, height: 200 },
      VERTICAL,
      WINDOW,
    );
    expect(spot.placement).toBe("left");
    expect(spot.left).toBe(600 - 40 - 8);
  });

  it("moves inside when neither side fits", () => {
    const spot = verticalToolbarPlacement(
      { x: 0, y: 0, width: 1000, height: 600 },
      VERTICAL,
      WINDOW,
    );
    expect(spot.placement).toBe("inside");
    expect(spot.left + VERTICAL.width).toBeLessThanOrEqual(WINDOW.width);
  });

  it("clamps the vertical position into the bounds", () => {
    const spot = verticalToolbarPlacement(
      { x: 100, y: 700, width: 200, height: 400 },
      VERTICAL,
      WINDOW,
    );
    expect(spot.top).toBe(800 - 250 - 8);
  });
});

/**
 * **这一组才是"超出屏幕自动调整"真正要过的关。**
 *
 * 上面那些用的边界是整个窗口，而贴图窗口的外框恒等于「内容 + 12×2 阴影 + 44 控件栏」——
 * 拿它当边界时右侧候选永远装得下，一次都不会翻边。第一版实现就是这么错的：函数本身
 * 有九条测试全绿，但真实调用永远走不到 `left`/`inside` 分支。所以这里用**后端给的
 * 可用范围**（窗口与屏幕工作区的交集，窗口局部坐标）来测，输入取自真实几何。
 */
describe("a pin window hanging off the screen edge", () => {
  // 内容 800×600 的贴图：外框 868×672（+12×2 阴影 +44 控件栏 / +48 工具条）。
  const media = { x: 12, y: 12, width: 800, height: 600 };
  const windowSize = { width: 868, height: 672 };

  it("keeps the toolbar on the right while the whole window is on screen", () => {
    const spot = verticalToolbarPlacement(media, VERTICAL, fullWindowBounds(windowSize));
    expect(spot.placement).toBe("right");
  });

  /**
   * 窗口右边 300 px 在屏幕外：工具条必须离开右侧，而且整条留在可用范围内。
   *
   * 落点是**内部**而不是左侧——这一条是真的、不是将就：贴图内容区左边只有 12 px 阴影，
   * 竖排工具条 38 px 宽，`left` 分支对贴图永远装不下。所以贴图的降级路径实际是
   * 右 → 内部（`left` 留给别的调用方，比如以后横排内容用同一个函数）。
   */
  it("leaves the right side when the right edge of the window is off screen", () => {
    const bounds = { x: 0, y: 0, width: 568, height: 672 };
    const spot = verticalToolbarPlacement(media, VERTICAL, bounds);
    expect(spot.placement).not.toBe("right");
    expect(spot.left).toBeGreaterThanOrEqual(bounds.x);
    expect(spot.left + VERTICAL.width).toBeLessThanOrEqual(bounds.x + bounds.width);
  });

  /** 内容区左边留够地方时才走 `left`：这条锁住那个分支本身没写错。 */
  it("flips left when the anchor has room on its left", () => {
    const roomy = { x: 200, y: 12, width: 300, height: 600 };
    const bounds = { x: 0, y: 0, width: 520, height: 672 };
    const spot = verticalToolbarPlacement(roomy, VERTICAL, bounds);
    expect(spot.placement).toBe("left");
    expect(spot.left).toBe(200 - 40 - 8);
  });

  /** 左右都被切掉（窄屏、窗口比可见区还宽）：只能压进内容里，而且不能超出可用范围。 */
  it("moves inside when both sides are off screen", () => {
    const bounds = { x: 40, y: 0, width: 500, height: 672 };
    const spot = verticalToolbarPlacement(media, VERTICAL, bounds);
    expect(spot.placement).toBe("inside");
    expect(spot.left).toBeGreaterThanOrEqual(bounds.x);
    expect(spot.left + VERTICAL.width).toBeLessThanOrEqual(bounds.x + bounds.width);
  });

  /** 窗口顶部在屏幕外（拖到屏幕上边缘）：可用范围带 y 偏移，工具条不能跑到它上面去。 */
  it("respects a vertical offset in the available bounds", () => {
    const bounds = { x: 0, y: 120, width: 868, height: 552 };
    const spot = verticalToolbarPlacement(media, VERTICAL, bounds);
    expect(spot.top).toBeGreaterThanOrEqual(bounds.y + 8);
  });

  /** 画布工具条同理：窗口下边在屏幕外时必须翻到内容上方。 */
  it("flips the horizontal toolbar above when the bottom is off screen", () => {
    const bounds = { x: 0, y: 0, width: 868, height: 400 };
    const spot = horizontalToolbarPlacement(media, HORIZONTAL, bounds);
    expect(spot.placement).not.toBe("below");
    expect(spot.top + HORIZONTAL.height).toBeLessThanOrEqual(bounds.y + bounds.height);
  });
});

describe("manual toolbar position", () => {
  it("keeps a dragged toolbar reachable", () => {
    expect(clampToolbarPosition({ left: -200, top: -80 }, VERTICAL, WINDOW)).toEqual({
      left: 8,
      top: 8,
    });
    expect(clampToolbarPosition({ left: 5000, top: 5000 }, VERTICAL, WINDOW)).toEqual({
      left: 1000 - 40 - 8,
      top: 800 - 250 - 8,
    });
    expect(clampToolbarPosition({ left: 120, top: 200 }, VERTICAL, WINDOW)).toEqual({
      left: 120,
      top: 200,
    });
  });

  /** 拖动也要受可用范围约束，不能拖到屏幕外那一半窗口上去。 */
  it("clamps a dragged toolbar into the on-screen part of the window", () => {
    const bounds = { x: 0, y: 0, width: 568, height: 672 };
    expect(clampToolbarPosition({ left: 800, top: 100 }, VERTICAL, bounds)).toEqual({
      left: 568 - 40 - 8,
      top: 100,
    });
  });

  it("degrades to the edge when the toolbar is larger than the bounds", () => {
    expect(
      clampToolbarPosition({ left: 50, top: 50 }, { width: 400, height: 400 }, fullWindowBounds({
        width: 200,
        height: 200,
      })),
    ).toEqual({ left: 8, top: 8 });
  });
});
