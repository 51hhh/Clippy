import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getCurrentWindowLabel: vi.fn(() => "pin-image-test"),
  startDraggingCurrentWindow: vi.fn(),
  pinApi: {
    get: vi.fn(),
    ready: vi.fn(),
    update: vi.fn(),
    copy: vi.fn(),
    save: vi.fn(),
    edit: vi.fn(),
    close: vi.fn(),
    onSharpened: vi.fn(),
    onAlreadyOpen: vi.fn(),
  },
}));

vi.mock("../js/api.ts", () => ({
  getCurrentWindowLabel: mocks.getCurrentWindowLabel,
  startDraggingCurrentWindow: mocks.startDraggingCurrentWindow,
}));
vi.mock("../react/pin/api.ts", () => ({ pinApi: mocks.pinApi }));

import * as i18n from "../i18n/i18n.js";
import { App } from "../react/pin/App.tsx";

const payload = {
  label: "pin-image-test",
  kind: "text",
  text: "Pinned text",
  imageBase64: null,
  contentWidth: 320,
  contentHeight: 180,
  scale: 1,
  opacity: 1,
  locked: false,
  above: false,
  canSave: true,
  position: null,
};

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function flushFrame() {
  await act(async () => {
    await new Promise((resolve) => requestAnimationFrame(resolve));
    await Promise.resolve();
  });
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("React pin app", () => {
  let root;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    i18n.init("en");
    for (const fn of Object.values(mocks.pinApi)) fn.mockReset();
    mocks.pinApi.ready.mockResolvedValue(undefined);
    mocks.pinApi.close.mockResolvedValue(undefined);
    mocks.pinApi.onSharpened.mockResolvedValue(() => {});
    mocks.pinApi.onAlreadyOpen.mockResolvedValue(() => {});
    mocks.pinApi.update.mockImplementation(async (_label, update) => ({ ...payload, ...update }));
    root = createRoot(document.getElementById("root"));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  it("closes a hidden window when its payload cannot be loaded", async () => {
    mocks.pinApi.get.mockRejectedValue(new Error("missing payload"));
    await act(async () => root.render(React.createElement(App)));
    await flush();

    expect(mocks.pinApi.close).toHaveBeenCalledWith("pin-image-test");
    expect(mocks.pinApi.ready).not.toHaveBeenCalled();
  });

  it("reports action failures instead of leaking rejected promises", async () => {
    mocks.pinApi.get.mockResolvedValue(payload);
    mocks.pinApi.copy.mockRejectedValue(new Error("copy failed"));
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const copy = document.querySelector('button[aria-label="Copy"]');
    await act(async () => copy.click());
    await flush();

    expect(document.querySelector(".pin-toast")?.textContent).toBe("Action failed");
  });

  it("rolls back an optimistic scale when the native resize fails", async () => {
    // 请求挂着不应答，这样"乐观值上屏"与"失败后回滚"两段各自可断言，
    // 不依赖 rAF 在哪一次 await 里恰好跑到。以前这里用 mockRejectedValue 立刻拒绝，
    // 于是 jsdom 的 rAF（定时器实现）在负载高时会在第一个 act 里就跑完、当场回滚，
    // 断言时序变成看机器快慢——三次里挂两次。
    const attempt = deferred();
    mocks.pinApi.get.mockResolvedValue(payload);
    mocks.pinApi.update.mockReturnValue(attempt.promise);
    await act(async () => root.render(React.createElement(App)));
    await flush();

    await act(async () => document.querySelector('button[aria-label="Zoom in"]').click());
    await flushFrame();
    await flush();
    expect(mocks.pinApi.update).toHaveBeenCalledWith("pin-image-test", { scale: 1.1 });
    expect(document.querySelector(".pin-scale")?.textContent).toBe("110");

    attempt.reject(new Error("resize failed"));
    await flush();

    expect(document.querySelector(".pin-scale")?.textContent).toBe("100");
    expect(document.querySelector(".pin-toast")?.textContent).toBe("Action failed");
  });

  /**
   * 后台算好的清晰版图片换进来（`pin/resample.rs` + `spawn_sharpen`）。
   *
   * 两条都要锁住：
   * 1. 事件到货后 `<img>` 真的换成了新图；
   * 2. 之后的 `update_pin` 应答**不会**把它换回原图。应答只带可变字段，前端拿
   *    `confirmedPinRef` 当基底合并——那份引用里的图片没跟着更新的话，用户滚一下滚轮
   *    刚变清楚的图就退回去了。
   */
  it("swaps in the sharpened image and keeps it across later updates", async () => {
    const urls = [];
    const createObjectURL = vi.fn((blob) => {
      const url = `blob:pin-${urls.length}`;
      urls.push({ url, blob });
      return url;
    });
    vi.stubGlobal("URL", Object.assign(Object.create(URL), {
      createObjectURL,
      revokeObjectURL: vi.fn(),
    }));
    // `update_pin` 的应答只有可变字段，不带图片（见 `PinState`）。
    mocks.pinApi.update.mockImplementation(async (label, update) => ({
      label,
      contentWidth: payload.contentWidth,
      contentHeight: payload.contentHeight,
      scale: update.scale ?? payload.scale,
      opacity: update.opacity ?? payload.opacity,
      locked: update.locked ?? payload.locked,
      position: null,
    }));
    let deliver;
    mocks.pinApi.onSharpened.mockImplementation(async (callback) => {
      deliver = callback;
      return () => {};
    });
    mocks.pinApi.get.mockResolvedValue({
      ...payload,
      kind: "image",
      text: null,
      // 1x1 透明 PNG
      imageBase64:
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNkAAIAAAoAAv/lxKUAAAAASUVORK5CYII=",
    });
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const original = document.querySelector(".pin-media img")?.getAttribute("src");
    expect(original).toBe("blob:pin-0");

    await act(async () => deliver({ label: "pin-image-test", imageBase64: "c2hhcnA=" }));
    await flush();
    const sharpened = document.querySelector(".pin-media img")?.getAttribute("src");
    expect(sharpened).toBe("blob:pin-1");

    // 缩放一次：应答里没有图片，合并后仍然必须是清晰版那份
    await act(async () => document.querySelector('button[aria-label="Zoom in"]').click());
    await flushFrame();
    await flush();
    expect(document.querySelector(".pin-media img")?.getAttribute("src")).toBe(sharpened);

    vi.unstubAllGlobals();
  });

  it("does not let an older failed resize replace a newer update", async () => {
    const first = deferred();
    const second = deferred();
    mocks.pinApi.get.mockResolvedValue(payload);
    mocks.pinApi.update
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const zoomIn = document.querySelector('button[aria-label="Zoom in"]');
    await act(async () => zoomIn.click());
    await flushFrame();
    await act(async () => zoomIn.click());
    await flushFrame();
    expect(document.querySelector(".pin-scale")?.textContent).toBe("120");

    first.reject(new Error("old resize failed"));
    await flush();
    expect(document.querySelector(".pin-scale")?.textContent).toBe("120");

    second.resolve({ ...payload, scale: 1.2 });
    await flush();
    expect(document.querySelector(".pin-scale")?.textContent).toBe("120");
  });

  /**
   * 又对同一个条目按了 Pin：闪一下外围边框。
   *
   * 一个条目只对应一个贴图窗口是刻意的（label 是 GNOME Shell 扩展的查找键），
   * 但"什么都不发生"是个坏反馈——那张贴图可能正被别的窗口压着。动画靠 class 驱动，
   * 所以连按两次必须先摘掉再挂上，否则第二次不重新播放。
   */
  it("flashes the border when the same clip is pinned again", async () => {
    let remind;
    mocks.pinApi.onAlreadyOpen.mockImplementation(async (callback) => {
      remind = callback;
      return () => {};
    });
    mocks.pinApi.get.mockResolvedValue(payload);
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const media = () => document.querySelector(".pin-media");
    expect(media()?.classList.contains("reminding")).toBe(false);

    await act(async () => remind());
    await flushFrame();
    expect(media()?.classList.contains("reminding")).toBe(true);

    // 再按一次：class 先摘掉（这一帧），下一帧再挂上，动画因此重新播放。
    await act(async () => remind());
    expect(media()?.classList.contains("reminding")).toBe(false);
    await flushFrame();
    expect(media()?.classList.contains("reminding")).toBe(true);
  });

  /**
   * 置顶是可开关的、默认关。
   *
   * 默认关这一条要锁住：贴图以前在建窗、平台适配、缩放三处都被无条件置顶，
   * 用户没有任何办法让它退回普通层。按钮的按下态也要断言——工具条只在悬停时出现，
   * 状态看不出来的话用户不知道自己现在是开还是关。
   */
  it("keeps pins out of the always-on-top layer until the pin button is pressed", async () => {
    mocks.pinApi.get.mockResolvedValue(payload);
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const button = () => document.querySelector('button[aria-label^="Keep above"], button[aria-label^="Stop keeping above"]');
    expect(button()?.getAttribute("aria-label")).toBe("Keep above other windows");
    expect(button()?.getAttribute("aria-pressed")).toBe("false");

    await act(async () => button().click());
    await flushFrame();
    await flush();
    expect(mocks.pinApi.update).toHaveBeenCalledWith("pin-image-test", { above: true });
    expect(button()?.getAttribute("aria-label")).toBe("Stop keeping above other windows");
    expect(button()?.getAttribute("aria-pressed")).toBe("true");

    // 再按一次必须真的退出置顶层，而不是只换图标。
    await act(async () => button().click());
    await flushFrame();
    await flush();
    expect(mocks.pinApi.update).toHaveBeenLastCalledWith("pin-image-test", { above: false });
    expect(button()?.getAttribute("aria-pressed")).toBe("false");
  });
});
