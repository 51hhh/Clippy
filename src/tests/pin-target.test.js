import { describe, expect, it, vi } from "vitest";
import { resolvePinTarget } from "../js/pin-target.js";

describe("global Pin shortcut target resolution", () => {
  it("pins the focused row when the panel actually holds focus", async () => {
    const fetchLatest = vi.fn();
    await expect(resolvePinTarget({ id: 7 }, fetchLatest, true)).resolves.toEqual({ id: 7 });
    expect(fetchLatest).not.toHaveBeenCalled();
  });

  // 用户报障"截图后 pin 出来的不是最新的"就是这一条。侧栏开着时失焦不隐藏窗口，
  // 前端也就不会 releaseMemory，于是焦点行活过整个截图流程；新截图插到第 0 行后
  // prependClip 按 id 把焦点跟着老条目挪到第 1 行，信它就贴出上一张图。
  // 面板没焦点 = 没有"用户正看着的行"，必须问后端。
  it("ignores a stale focused row while the panel is not focused", async () => {
    const fetchLatest = vi.fn().mockResolvedValue([{ id: 42 }]);
    await expect(resolvePinTarget({ id: 7 }, fetchLatest, false)).resolves.toEqual({ id: 42 });
    expect(fetchLatest).toHaveBeenCalledTimes(1);
  });

  // 漏传第三个参数时退化成"问后端"：慢一点，但绝不会贴错。
  it("defaults to asking the backend when the caller omits the focus flag", async () => {
    const fetchLatest = vi.fn().mockResolvedValue([{ id: 42 }]);
    await expect(resolvePinTarget({ id: 7 }, fetchLatest)).resolves.toEqual({ id: 42 });
    expect(fetchLatest).toHaveBeenCalledTimes(1);
  });

  // 入库由 watcher 完成，前端列表缓存要等 clip-added 才更新，所以这条路读库不读缓存。
  it("asks the backend for the newest clip instead of trusting the cached list", async () => {
    const fetchLatest = vi.fn().mockResolvedValue([{ id: 42 }]);
    await expect(resolvePinTarget(null, fetchLatest, true)).resolves.toEqual({ id: 42 });
    expect(fetchLatest).toHaveBeenCalledTimes(1);
  });

  it("resolves to null when history is empty", async () => {
    await expect(resolvePinTarget(null, async () => [], true)).resolves.toBeNull();
    await expect(resolvePinTarget(null, async () => undefined, true)).resolves.toBeNull();
  });

  it("propagates a backend failure so the caller can log it", async () => {
    await expect(resolvePinTarget(null, async () => {
      throw new Error("db down");
    }, true)).rejects.toThrow("db down");
  });
});
