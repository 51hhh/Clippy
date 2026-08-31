import { describe, expect, it } from "vitest";

import { isPixelExact, pinImageRendering } from "../react/pin/rendering.ts";
import { pointerStillHeld } from "../react/pin/gestures.ts";

/**
 * 这一组锁住的是"什么时候该让 WebKit 用最近邻搬图"。
 *
 * 结论与实测数字见 `react/pin/rendering.ts`：屏上一个图片像素正好一个设备像素时，
 * 最近邻比默认平滑清晰约 3.7 dB；一旦不是原尺寸，最近邻反过来要差 3~4.5 dB。
 * 所以判据只能是"显示尺寸 × 真实缩放 == 图片像素数"。
 */
describe("pin image rendering filter", () => {
  /** 分数缩放桌面的典型值：1200x900 的图按原物理尺寸贴在 1.5 倍屏上。 */
  const fractional = {
    cssWidth: 800,
    cssHeight: 600,
    pixelWidth: 1200,
    pixelHeight: 900,
    deviceScale: 1.5,
  };

  it("uses nearest neighbour when one image pixel lands on one device pixel", () => {
    expect(isPixelExact(fractional)).toBe(true);
    expect(pinImageRendering(fractional)).toBe("pixelated");
  });

  /** X11 / 未缩放桌面：CSS 像素就是设备像素，同样是 1:1。 */
  it("also recognizes the unscaled desktop as pixel exact", () => {
    expect(pinImageRendering({
      cssWidth: 1200,
      cssHeight: 900,
      pixelWidth: 1200,
      pixelHeight: 900,
      deviceScale: 1,
    })).toBe("pixelated");
  });

  /**
   * 关键的一条：判据**不能**写成 `scale === 1`。
   * 全屏截图贴出来时内容会被 `origin_content_size` 按工作区缩小，`scale` 仍然是 1，
   * 那种情况按最近邻画就是实测里最差的 26.37 dB。
   */
  it("falls back to smoothing for a shrunk full-screen pin even at scale 1", () => {
    // 2560x1440 的整屏图缩到工作区能放下的 1400x787 逻辑像素
    expect(pinImageRendering({
      cssWidth: 1400,
      cssHeight: 787,
      pixelWidth: 3840,
      pixelHeight: 2160,
      deviceScale: 1.5,
    })).toBe("auto");
  });

  it("falls back to smoothing once the user zooms away from the original size", () => {
    expect(pinImageRendering({ ...fractional, cssWidth: 960, cssHeight: 720 })).toBe("auto");
    expect(pinImageRendering({ ...fractional, cssWidth: 480, cssHeight: 360 })).toBe("auto");
  });

  /** 外框尺寸要取整到整数逻辑像素，允许不到一个设备像素的零头。 */
  it("tolerates less than one device pixel of rounding", () => {
    expect(isPixelExact({ ...fractional, cssWidth: 800 + 0.6 / 1.5 })).toBe(true);
    expect(isPixelExact({ ...fractional, cssWidth: 802 })).toBe(false);
  });

  /** 拿不到有效数字时一律走默认平滑：糊一点也好过满屏锯齿。 */
  it("never picks nearest neighbour from bad numbers", () => {
    expect(isPixelExact({ ...fractional, deviceScale: 0 })).toBe(false);
    expect(isPixelExact({ ...fractional, deviceScale: Number.NaN })).toBe(false);
    expect(isPixelExact({ ...fractional, pixelWidth: 0, pixelHeight: 0 })).toBe(false);
    expect(isPixelExact({ ...fractional, cssHeight: Number.POSITIVE_INFINITY })).toBe(false);
  });
});

/**
 * 拖动判据只看"主键还按着没有"。回归的是那个"第一下能拖、第二下拖不动"的毛病：
 * Wayland 上 `startDragging` 之后指针被合成器抓走，迟到的 `pointercancel` 会落在
 * 下一次 `pointerdown` 之后，任何跨事件的记账都会被它抹掉。
 */
describe("pointerStillHeld", () => {
  it("is true only while the primary button is down", () => {
    expect(pointerStillHeld({ buttons: 1 })).toBe(true);
    // 主键 + 右键同时按着也算
    expect(pointerStillHeld({ buttons: 3 })).toBe(true);
    expect(pointerStillHeld({ buttons: 0 })).toBe(false);
    // 只按着右键/中键：不是拖窗口
    expect(pointerStillHeld({ buttons: 2 })).toBe(false);
    expect(pointerStillHeld({ buttons: 4 })).toBe(false);
  });
});
