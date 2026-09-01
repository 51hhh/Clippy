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
    saveCanvas: vi.fn(),
    toolbarBounds: vi.fn(),
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

/** 1x1 透明 PNG。 */
const TINY_PNG =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNkAAIAAAoAAv/lxKUAAAAASUVORK5CYII=";

/** jsdom 没有 PointerEvent，用 MouseEvent 顶替并补上画布要读的那几个字段。 */
function pointer(type, x, y) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y });
  Object.defineProperty(event, "pointerId", { value: 1 });
  Object.defineProperty(event, "buttons", { value: type === "pointerup" ? 0 : 1 });
  return event;
}

/**
 * 打开画布并画一笔。
 *
 * jsdom 不会真的加载图片、也没有布局，所以要手动派发 load、给出像素尺寸，
 * 并给画布一个真的 `getBoundingClientRect`——交互层靠它把指针位置换算成图片坐标。
 */
async function drawOneStroke() {
  const image = document.querySelector(".pin-media img");
  Object.defineProperty(image, "naturalWidth", { value: 320, configurable: true });
  Object.defineProperty(image, "naturalHeight", { value: 180, configurable: true });
  await act(async () => image.dispatchEvent(new Event("load")));
  await flush();

  await act(async () => document.querySelector('button[aria-label="Draw on image"]').click());
  await flush();

  const canvas = document.querySelector(".pin-canvas");
  canvas.getBoundingClientRect = () => ({
    left: 0, top: 0, right: 320, bottom: 180, width: 320, height: 180, x: 0, y: 0,
  });
  canvas.setPointerCapture = () => {};
  canvas.releasePointerCapture = () => {};
  await act(async () => {
    canvas.dispatchEvent(pointer("pointerdown", 10, 10));
    canvas.dispatchEvent(pointer("pointermove", 60, 70));
    canvas.dispatchEvent(pointer("pointerup", 60, 70));
  });
  await flushFrame();
  await flush();
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
    mocks.pinApi.saveCanvas.mockResolvedValue("/tmp/pin.png");
    mocks.pinApi.save.mockResolvedValue("/tmp/pin.png");
    // 默认：整个窗口都在屏幕内（宽高取自 payload 的内容尺寸 + 边距）。
    mocks.pinApi.toolbarBounds.mockResolvedValue({ x: 0, y: 0, width: 388, height: 252 });
    mocks.pinApi.update.mockImplementation(async (_label, update) => ({ ...payload, ...update }));
    // 产品代码会对它 `.catch()`，必须回 Promise（Wayland 上这条会真的失败，
    // 所以那个 catch 不是多余的）。
    mocks.startDraggingCurrentWindow.mockReset();
    mocks.startDraggingCurrentWindow.mockResolvedValue(undefined);
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
      imageBase64: TINY_PNG,
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
   * 拖工具条不能把整个贴图窗口拖走。
   *
   * 窗口拖动的判据是"主键按着 + 位移够大 + 落点不在 `[data-pin-controls]` 里"
   * （`gestures.ts` 刻意不跨事件记账）。工具条跟着指针走，指针很容易落到工具条外面，
   * 那一刻判据当场成立——这就是那个 bug。pointer capture 让 target 钉在把手上，
   * `isToolbarDragging()` 是捕获没生效时的第二道闸。
   */
  it("never drags the window while the toolbar handle is being dragged", async () => {
    mocks.pinApi.get.mockResolvedValue(payload);
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const grip = document.querySelector(".pin-tool-grip");
    grip.setPointerCapture = () => {};
    grip.releasePointerCapture = () => {};
    await act(async () => grip.dispatchEvent(pointer("pointerdown", 300, 40)));

    // 指针拖到工具条外面（落点是 .pin-root），位移远超阈值。
    await act(async () => {
      window.dispatchEvent(pointer("pointermove", 120, 300));
      window.dispatchEvent(pointer("pointermove", 60, 400));
    });
    expect(mocks.startDraggingCurrentWindow).not.toHaveBeenCalled();

    // 松手之后普通拖动要恢复，否则窗口从此拖不动。
    await act(async () => window.dispatchEvent(pointer("pointerup", 60, 400)));
    await act(async () => {
      document.querySelector(".pin-root").dispatchEvent(pointer("pointerdown", 200, 200));
      window.dispatchEvent(pointer("pointermove", 260, 260));
    });
    expect(mocks.startDraggingCurrentWindow).toHaveBeenCalled();
  });

  /**
   * 确认框开着时 Esc = 取消，而且背后的交互要被挡住。
   *
   * 以前 Esc 无条件走 requestClose()，那时 dirty 仍为真，于是只是把 closePrompt 又设一次
   * true——框不关、也没别的反应，用户会觉得按键失灵。`role="dialog"` 也得名副其实：
   * 开着时滚轮不该还在改缩放。
   */
  it("closes the prompt on Escape and blocks interaction behind it", async () => {
    mocks.pinApi.get.mockResolvedValue({
      ...payload,
      kind: "image",
      text: null,
      imageBase64: TINY_PNG,
    });
    await act(async () => root.render(React.createElement(App)));
    await flush();
    await drawOneStroke();

    await act(async () => document.querySelector('button[aria-label="Close"]').click());
    await flush();
    expect(document.querySelector(".pin-close-prompt")).not.toBeNull();

    // 滚轮被挡住：缩放不变。
    await act(async () => {
      window.dispatchEvent(new WheelEvent("wheel", { deltaY: -120, cancelable: true }));
    });
    await flushFrame();
    await flush();
    expect(document.querySelector(".pin-scale")?.textContent).toBe("100");

    // Esc 收框，不关窗。
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await flush();
    expect(document.querySelector(".pin-close-prompt")).toBeNull();
    expect(mocks.pinApi.close).not.toHaveBeenCalled();
  });

  /** 两条消息只占一个 toast 位（CSS 把它定位在右下角），出错优先。 */
  it("shows only one toast at a time", async () => {
    mocks.pinApi.get.mockResolvedValue(payload);
    mocks.pinApi.copy.mockRejectedValue(new Error("copy failed"));
    await act(async () => root.render(React.createElement(App)));
    await flush();

    await act(async () => document.querySelector('button[aria-label="Save image"]').click());
    await flush();
    expect(document.querySelectorAll(".pin-toast")).toHaveLength(1);
    expect(document.querySelector(".pin-toast").textContent).toBe("Saved");

    // 出错之后仍然只有一个，而且显示的是错误。
    await act(async () => document.querySelector('button[aria-label="Copy"]').click());
    await flush();
    expect(document.querySelectorAll(".pin-toast")).toHaveLength(1);
    expect(document.querySelector(".pin-toast").textContent).toBe("Action failed");
  });

  /**
   * 右键接管：WebKit 自带的网页菜单（重新加载/检查元素）已经在 GTK 层关掉了，
   * 这里断言腾出来的右键真的接成了快速操作，而且默认行为被拦住
   * （那一层万一没生效，也不能弹出浏览器菜单）。
   */
  it("replaces the browser context menu with pin actions", async () => {
    mocks.pinApi.get.mockResolvedValue(payload);
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    await act(async () => {
      document.querySelector(".pin-root").dispatchEvent(event);
    });
    expect(event.defaultPrevented).toBe(true);

    const labels = [...document.querySelectorAll(".pin-context-menu button")].map(
      (button) => button.textContent,
    );
    // 文本贴图没有画布与保存（canSave 为 true 但 kind 是 text 时仍给保存，
    // 这里用的 payload 是 text + canSave，所以画布项在）。
    expect(labels).toContain("Keep above other windows");
    expect(labels).toContain("Lock position");
    expect(labels).toContain("Close");

    // 选一项：动作发出去，菜单收起。
    const above = [...document.querySelectorAll(".pin-context-menu button")].find(
      (button) => button.textContent === "Keep above other windows",
    );
    await act(async () => above.click());
    await flushFrame();
    await flush();
    expect(mocks.pinApi.update).toHaveBeenCalledWith("pin-image-test", { above: true });
    expect(document.querySelector(".pin-context-menu")).toBeNull();
  });

  /**
   * 画布按钮开合，且**没画东西时关窗不问**。
   *
   * "关闭前提醒保存"只在画布真的脏了的时候才该出现，否则每次关贴图都要多点一下。
   */
  it("opens the drawing canvas and closes without prompting when nothing was drawn", async () => {
    mocks.pinApi.get.mockResolvedValue({
      ...payload,
      kind: "image",
      text: null,
      imageBase64: TINY_PNG,
    });
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const canvasButton = () => document.querySelector('button[aria-label="Draw on image"], button[aria-label="Close drawing tools"]');
    expect(document.querySelector(".pin-canvas-toolbar")).toBeNull();

    await act(async () => canvasButton().click());
    await flush();
    expect(document.querySelector(".pin-canvas-toolbar")).not.toBeNull();
    expect(canvasButton()?.getAttribute("aria-pressed")).toBe("true");

    // 一笔都没画：关窗直接走，不弹询问。
    await act(async () => document.querySelector('button[aria-label="Close"]').click());
    await flush();
    expect(document.querySelector(".pin-close-prompt")).toBeNull();
    expect(mocks.pinApi.close).toHaveBeenCalledWith("pin-image-test");
  });

  /**
   * 画过东西再关窗：必须先问，而且三条出路都要在。
   *
   * 这是"画布产物不写回条目"的必然结果——不问就等于静默丢掉用户画的东西
   * （`copy_pin`/`save_pin` 交付的一直是原图，见 `usePinCanvas`）。
   */
  it("asks before closing when the canvas has unsaved drawing", async () => {
    mocks.pinApi.get.mockResolvedValue({
      ...payload,
      kind: "image",
      text: null,
      imageBase64: TINY_PNG,
    });
    await act(async () => root.render(React.createElement(App)));
    await flush();

    await drawOneStroke();

    await act(async () => document.querySelector('button[aria-label="Close"]').click());
    await flush();
    const prompt = document.querySelector(".pin-close-prompt");
    expect(prompt).not.toBeNull();
    expect([...prompt.querySelectorAll("button")].map((button) => button.textContent)).toEqual([
      "Save and close",
      "Discard",
      "Cancel",
    ]);
    // 问的时候绝不能已经关掉了
    expect(mocks.pinApi.close).not.toHaveBeenCalled();

    // 取消：窗口留着，画的东西也留着。
    await act(async () => [...prompt.querySelectorAll("button")][2].click());
    await flush();
    expect(document.querySelector(".pin-close-prompt")).toBeNull();
    expect(mocks.pinApi.close).not.toHaveBeenCalled();

    // 不保存：直接关，不碰 saveCanvas。
    await act(async () => document.querySelector('button[aria-label="Close"]').click());
    await flush();
    await act(async () => [...document.querySelectorAll(".pin-close-prompt button")][1].click());
    await flush();
    expect(mocks.pinApi.saveCanvas).not.toHaveBeenCalled();
    expect(mocks.pinApi.close).toHaveBeenCalledWith("pin-image-test");
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
