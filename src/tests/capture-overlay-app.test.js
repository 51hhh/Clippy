import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getCurrentWindowLabel: vi.fn(() => "capture-overlay-session-1-0"),
  getConfig: vi.fn(async () => ({ language: "en" })),
  overlayApi: {
    get: vi.fn(),
    image: vi.fn(),
    frame: vi.fn(),
    ready: vi.fn(),
    cancel: vi.fn(),
    commit: vi.fn(),
    translate: vi.fn(),
    copyText: vi.fn(),
    openLongshot: vi.fn(),
  },
}));

vi.mock("../js/api.ts", () => ({
  getCurrentWindowLabel: mocks.getCurrentWindowLabel,
  getConfig: mocks.getConfig,
}));
vi.mock("../react/capture-overlay/api.ts", () => ({ overlayApi: mocks.overlayApi }));

import * as i18n from "../i18n/i18n.js";
import { App } from "../react/capture-overlay/App.tsx";

const basePayload = {
  sessionId: "session-1",
  monitorId: 0,
  // 故意不是 (0,0)：这块屏在桌面坐标里靠右，能验出"选区 + 显示器偏移"有没有做对
  logicalX: 1920,
  logicalY: 24,
  logicalWidth: 200,
  logicalHeight: 150,
  pixelWidth: 200,
  pixelHeight: 150,
  windows: [{ x: 40, y: 30, width: 60, height: 50, title: "editor" }],
  probeHint: false,
};

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function pointer(type, x, y) {
  const root = document.querySelector(".overlay-root");
  root.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX: x, clientY: y, button: 0 }));
}

async function drag(from, to) {
  await act(async () => {
    pointer("pointerdown", from.x, from.y);
    pointer("pointermove", to.x, to.y);
    pointer("pointerup", to.x, to.y);
  });
  await flush();
}

/** 当前选区框在 DOM 上的几何，覆盖层用内联样式定位。 */
function selectionRect() {
  const node = document.querySelector(".selection");
  if (!node) return null;
  const px = (value) => Number.parseFloat(value);
  return {
    x: px(node.style.left),
    y: px(node.style.top),
    width: px(node.style.width),
    height: px(node.style.height),
  };
}

const button = (label) => document.querySelector(`button[aria-label="${label}"]`);

function protocolImage(width, height) {
  const image = document.createElement("img");
  Object.defineProperties(image, {
    naturalWidth: { value: width },
    naturalHeight: { value: height },
  });
  return image;
}

function changeRange(input, value) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  setValue.call(input, String(value));
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("capture overlay app", () => {
  let root;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    i18n.init("en");
    // jsdom 缺这些：覆盖层的指针捕获与画布上下文都会直接抛错。
    Element.prototype.setPointerCapture = () => {};
    // jsdom 没有 canvas 后端，给底图与标注层一个万能空实现：
    // 任何方法都是 no-op，任何属性都可写，绘制结果反正不参与断言。
    HTMLCanvasElement.prototype.getContext = () =>
      new Proxy(
        {},
        {
          get: (target, key) => (target[key] ??= () => {}),
          set: () => true,
        },
      );
    globalThis.ImageData = class {
      constructor(data, width, height) {
        Object.assign(this, { data, width, height });
      }
    };
    for (const fn of Object.values(mocks.overlayApi)) fn.mockReset();
    mocks.overlayApi.ready.mockResolvedValue(undefined);
    mocks.overlayApi.image.mockResolvedValue(
      protocolImage(basePayload.pixelWidth, basePayload.pixelHeight),
    );
    mocks.overlayApi.frame.mockResolvedValue(
      new ArrayBuffer(basePayload.pixelWidth * basePayload.pixelHeight * 4),
    );
    mocks.overlayApi.commit.mockResolvedValue({ action: "copy", path: null, pinLabel: null });
    mocks.overlayApi.translate.mockResolvedValue({
      sourceText: "source",
      translatedText: "translated",
      provider: "offline",
      detectedSourceLanguage: null,
    });
    mocks.overlayApi.openLongshot.mockResolvedValue({ label: "longshot-controller-session-1" });
    root = createRoot(document.getElementById("root"));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  async function mount(overrides = {}) {
    const payload = { ...basePayload, ...overrides };
    mocks.overlayApi.get.mockResolvedValue(payload);
    mocks.overlayApi.image.mockResolvedValue(protocolImage(payload.pixelWidth, payload.pixelHeight));
    // raw IPC 是图像协议失败时的兜底，字节数仍必须正好是 4 × 像素数。
    mocks.overlayApi.frame.mockResolvedValue(
      new ArrayBuffer(payload.pixelWidth * payload.pixelHeight * 4),
    );
    await act(async () => root.render(React.createElement(App)));
    await flush();
  }

  it("shows the protocol image at native resolution without allocating the canvas first", async () => {
    await mount({ logicalWidth: 100, logicalHeight: 50, pixelWidth: 320, pixelHeight: 180 });
    const image = document.querySelector(".overlay-frame");
    const canvas = document.querySelector(".overlay-canvas");
    expect([image.naturalWidth, image.naturalHeight]).toEqual([320, 180]);
    expect(canvas.classList.contains("is-idle")).toBe(true);
    // 浏览器默认尺寸仍是 300×150，说明首帧没有为冻结图调整/分配 Canvas backing store。
    expect([canvas.width, canvas.height]).toEqual([300, 150]);
  });

  it("prefers the native image protocol without requesting the raw IPC frame", async () => {
    await mount();
    expect(mocks.overlayApi.image).toHaveBeenCalledWith("capture-overlay-session-1-0");
    expect(mocks.overlayApi.frame).not.toHaveBeenCalled();
  });

  it("allocates and shows the native-size canvas only after a real image adjustment", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });
    await act(async () => button("Image").click());
    await act(async () => document.querySelector(".overlay-toggle input").click());

    const canvas = document.querySelector(".overlay-canvas");
    expect(document.querySelector(".overlay-frame-host").classList.contains("is-composited"))
      .toBe(true);
    expect(canvas.classList.contains("is-idle")).toBe(false);
    expect([canvas.width, canvas.height]).toEqual([basePayload.pixelWidth, basePayload.pixelHeight]);
  });

  it("falls back to raw IPC when the native image protocol is unavailable", async () => {
    mocks.overlayApi.get.mockResolvedValue(basePayload);
    mocks.overlayApi.image.mockRejectedValue(new Error("protocol unavailable"));
    mocks.overlayApi.frame.mockResolvedValue(
      new ArrayBuffer(basePayload.pixelWidth * basePayload.pixelHeight * 4),
    );
    await act(async () => root.render(React.createElement(App)));
    await flush();

    expect(mocks.overlayApi.frame).toHaveBeenCalledWith("capture-overlay-session-1-0");
    expect(mocks.overlayApi.ready).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".overlay-canvas").classList.contains("is-idle")).toBe(false);
    expect(document.querySelector(".overlay-error")).toBeNull();
  });

  it("shows the toolbar next to a dragged selection instead of finishing", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    // 松手不提交：工具条留在选区旁边等用户标注
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();
    expect(selectionRect()).toEqual({ x: 10, y: 10, width: 90, height: 70 });
    expect(document.querySelector(".overlay-toolbar")).not.toBeNull();
    expect(button("Select area")).not.toBeNull();
    expect(button("Blur")).not.toBeNull();
    expect(button("Translate selection")).not.toBeNull();
  });

  it("quick-picks the hovered window on a plain click", async () => {
    await mount();
    await drag({ x: 50, y: 40 }, { x: 51, y: 40 });

    expect(selectionRect()).toEqual({ x: 40, y: 30, width: 60, height: 50 });
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();
  });

  it("takes the whole screen when clicking empty space", async () => {
    await mount({ windows: [] });
    await drag({ x: 150, y: 120 }, { x: 151, y: 120 });

    expect(selectionRect()).toEqual({ x: 0, y: 0, width: 200, height: 150 });
    expect(document.querySelector(".overlay-toolbar")).not.toBeNull();
  });

  it("keeps re-framing possible after a full-screen selection", async () => {
    await mount({ windows: [] });
    await drag({ x: 150, y: 120 }, { x: 151, y: 120 });
    // 铺满全屏时"选区内部"让位给重新框选，否则点一下取整屏后就再也框不小了
    await drag({ x: 20, y: 20 }, { x: 80, y: 60 });

    expect(selectionRect()).toEqual({ x: 20, y: 20, width: 60, height: 40 });
  });

  it("still resizes the selection from its handles after it is committed", async () => {
    await mount();
    await drag({ x: 20, y: 20 }, { x: 100, y: 90 });
    // 右下角手柄：拖动它改框，而不是新框一个
    await drag({ x: 100, y: 90 }, { x: 140, y: 120 });

    expect(selectionRect()).toEqual({ x: 20, y: 20, width: 120, height: 100 });
  });

  it("submits the selection and renderer document when copy is pressed", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Copy").click());
    await flush();

    // origin 是选区在**桌面**逻辑坐标里的矩形（选区坐标 + 这块屏的 logicalX/logicalY）：
    // 贴图靠它回到原位，复制时后端也记一份，之后从历史里 Pin 同一张图仍能回到原处
    expect(mocks.overlayApi.commit).toHaveBeenCalledWith(
      "copy",
      { x: 10, y: 10, width: 100, height: 80, sessionId: "session-1", monitorId: 0 },
      {
        rendererVersion: 2,
        sourceWidth: 200,
        sourceHeight: 150,
        annotations: [],
        adjustments: {
          grayscale: false,
          brightness: 0,
          contrast: 0,
          saturation: 0,
          cornerRadius: 0,
        },
      },
      { x: 1930, y: 34, width: 100, height: 80 },
    );
  });

  it("routes save and pin through the same commit path", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => button("Pin").click());
    await flush();
    expect(mocks.overlayApi.commit).toHaveBeenCalledWith(
      "pin",
      { x: 10, y: 10, width: 100, height: 80, sessionId: "session-1", monitorId: 0 },
      expect.objectContaining({ rendererVersion: 2, sourceWidth: 200, sourceHeight: 150 }),
      { x: 1930, y: 34, width: 100, height: 80 },
    );
  });

  it("opens one label-free longshot controller from a clean, complete selection", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => {
      button("Long screenshot").click();
      // React 尚未来得及重渲染 disabled；同步 ref 仍必须挡住这一击。
      button("Long screenshot").click();
    });
    await flush();

    expect(mocks.overlayApi.openLongshot).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.openLongshot).toHaveBeenCalledWith({
      sessionId: "session-1",
      monitorId: 0,
      x: 10,
      y: 10,
      width: 100,
      height: 80,
    });
    expect(mocks.overlayApi.cancel).not.toHaveBeenCalled();
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();
  });

  it("does not open Longshot after a same-tick ordinary copy wins", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => {
      button("Copy").click();
      button("Long screenshot").click();
    });
    await flush();

    expect(mocks.overlayApi.commit).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.openLongshot).not.toHaveBeenCalled();
    expect(button("Long screenshot").disabled).toBe(true);
  });

  it("does not open Longshot after a same-tick translation starts", async () => {
    let resolveTranslation;
    mocks.overlayApi.translate.mockReturnValue(new Promise((resolve) => (resolveTranslation = resolve)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => {
      button("Translate selection").click();
      button("Long screenshot").click();
    });
    await flush();

    expect(mocks.overlayApi.translate).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.openLongshot).not.toHaveBeenCalled();
    expect(button("Long screenshot").disabled).toBe(true);

    await act(async () => resolveTranslation({
      sourceText: "source",
      translatedText: "translated",
      provider: "offline",
      detectedSourceLanguage: null,
    }));
  });

  it("freezes edits, redo, and ordinary IPC while opening the controller", async () => {
    let rejectOpen;
    mocks.overlayApi.openLongshot.mockReturnValue(new Promise((_, reject) => (rejectOpen = reject)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Pen").click());
    await drag({ x: 24, y: 24 }, { x: 56, y: 56 });
    await act(async () => button("Undo").click());
    await act(async () => button("Image").click());
    const before = selectionRect();

    await act(async () => button("Long screenshot").click());
    await act(async () => {
      // 此时工具仍是 Pen：根画布路由必须在已捕获的绘制路径前同步拦住它。
      pointer("pointerdown", 20, 20);
      pointer("pointermove", 160, 130);
      pointer("pointerup", 160, 130);
      document.querySelector(".overlay-root").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      button("Redo").click();
      document.querySelector(".overlay-toggle input").click();
      button("Copy").click();
      button("Translate selection").click();
      button("Cancel").click();
    });
    await flush();

    expect(selectionRect()).toEqual(before);
    expect(button("Pen").disabled).toBe(true);
    expect(button("Redo").disabled).toBe(true);
    expect(document.querySelector(".overlay-toggle input").disabled).toBe(true);
    expect(button("Copy").disabled).toBe(true);
    expect(button("Translate selection").disabled).toBe(true);
    expect(button("Cancel").disabled).toBe(true);
    expect(mocks.overlayApi.openLongshot).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();
    expect(mocks.overlayApi.translate).not.toHaveBeenCalled();
    expect(mocks.overlayApi.copyText).not.toHaveBeenCalled();
    expect(mocks.overlayApi.cancel).not.toHaveBeenCalled();

    await act(async () => rejectOpen(new Error("controller unavailable")));
    await flush();
    expect(selectionRect()).toEqual(before);
    expect(button("Long screenshot").disabled).toBe(false);
    expect(button("Redo").disabled).toBe(false);
    expect(document.querySelector(".overlay-toggle input").checked).toBe(false);
  });

  it("keeps ordinary output available after open accepts while latching Longshot", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => button("Long screenshot").click());
    await flush();

    expect(button("Long screenshot").disabled).toBe(true);
    expect(button("Copy").disabled).toBe(false);
    expect(button("Cancel").disabled).toBe(false);
    expect(mocks.overlayApi.cancel).not.toHaveBeenCalled();
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();

    await act(async () => button("Copy").click());
    await flush();
    expect(mocks.overlayApi.commit).toHaveBeenCalledTimes(1);
  });

  it("keeps Longshot disabled for image edits without blocking ordinary output", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Image").click());
    await act(async () => document.querySelector(".overlay-toggle input").click());

    expect(button("Long screenshot").disabled).toBe(true);
    expect(button("Long screenshot").getAttribute("aria-description")).toBe(
      "Long screenshot starts from the original selection and cannot include current annotations or image adjustments.",
    );
    expect(button("Pin").disabled).toBe(false);

    await act(async () => document.querySelector(".overlay-toggle input").click());
    expect(button("Long screenshot").disabled).toBe(false);
  });

  it("treats nonzero corner rounding as a Longshot-dirty image edit", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Image").click());
    const corners = document.querySelector('input[aria-label="Corners"]');

    await act(async () => changeRange(corners, 24));
    expect(button("Long screenshot").disabled).toBe(true);
    expect(button("Long screenshot").getAttribute("aria-description")).toBe(
      "Long screenshot starts from the original selection and cannot include current annotations or image adjustments.",
    );
    expect(button("Copy").disabled).toBe(false);

    await act(async () => changeRange(corners, 0));
    expect(button("Long screenshot").disabled).toBe(false);
  });

  it("disables Longshot for annotations and re-enables it after undo", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Pen").click());
    await drag({ x: 24, y: 24 }, { x: 56, y: 56 });

    expect(button("Long screenshot").disabled).toBe(true);
    expect(button("Undo").disabled).toBe(false);

    await act(async () => button("Undo").click());
    expect(button("Long screenshot").disabled).toBe(false);
  });

  it("treats an in-progress annotation as dirty before it reaches history", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Pen").click());
    await act(async () => pointer("pointerdown", 24, 24));
    await flush();

    expect(button("Long screenshot").disabled).toBe(true);

    await act(async () => pointer("pointerup", 24, 24));
    await flush();
    expect(button("Long screenshot").disabled).toBe(false);
  });

  it("recovers from a rejected open without cancelling the ordinary session", async () => {
    mocks.overlayApi.openLongshot
      .mockRejectedValueOnce(new Error("raw controller build failure"))
      .mockResolvedValueOnce({ label: "longshot-controller-session-1" });
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => button("Long screenshot").click());
    await flush();

    expect(document.querySelector(".overlay-error")?.textContent).toBe(
      "Could not open the long screenshot controller. Please try again.",
    );
    expect(document.body.textContent).not.toContain("raw controller build failure");
    expect(button("Long screenshot").disabled).toBe(false);
    expect(mocks.overlayApi.cancel).not.toHaveBeenCalled();
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();

    await act(async () => button("Long screenshot").click());
    await flush();
    expect(mocks.overlayApi.openLongshot).toHaveBeenCalledTimes(2);
    expect(button("Long screenshot").disabled).toBe(true);
  });

  it("blocks a stale translation-copy click in the same tick as Longshot", async () => {
    let resolveOpen;
    mocks.overlayApi.openLongshot.mockReturnValue(new Promise((resolve) => (resolveOpen = resolve)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Translate selection").click());
    await flush();

    expect(document.querySelector(".translation-copy")).not.toBeNull();
    await act(async () => {
      button("Long screenshot").click();
      document.querySelector(".translation-copy").click();
    });
    await flush();

    expect(mocks.overlayApi.openLongshot).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.copyText).not.toHaveBeenCalled();
    await act(async () => resolveOpen({ label: "longshot-controller-session-1" }));
  });

  it("ignores late controller rejection after unmount without a compensating IPC", async () => {
    let rejectOpen;
    const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.overlayApi.openLongshot.mockReturnValue(new Promise((_, reject) => (rejectOpen = reject)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Long screenshot").click());
    await act(async () => root.unmount());
    await act(async () => rejectOpen(new Error("late failure")));
    await flush();

    expect(warning).not.toHaveBeenCalled();
    expect(mocks.overlayApi.cancel).not.toHaveBeenCalled();
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();
    warning.mockRestore();
  });

  it("ignores late controller resolution after unmount without a compensating IPC", async () => {
    let resolveOpen;
    mocks.overlayApi.openLongshot.mockReturnValue(new Promise((resolve) => (resolveOpen = resolve)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Long screenshot").click());
    await act(async () => root.unmount());
    await act(async () => resolveOpen({ label: "longshot-controller-session-1" }));
    await flush();

    expect(mocks.overlayApi.cancel).not.toHaveBeenCalled();
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();
  });

  it("drops the selection on right click so a new area can be framed", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    await act(async () => {
      document
        .querySelector(".overlay-root")
        .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
    });
    expect(selectionRect()).toBeNull();
    expect(document.querySelector(".overlay-toolbar")).toBeNull();
  });

  it("says window picking is unavailable when no window geometry arrived", async () => {
    await mount({ windows: [] });
    expect(document.querySelector(".overlay-hint")?.textContent)
      .toBe("Window picking unavailable in this session — drag to select an area");
  });

  it("keeps the overlay clean once window geometry is available", async () => {
    await mount();
    expect(document.querySelector(".overlay-hint")).toBeNull();
  });

  // GNOME Wayland 上速选不是"用不了"而是"缺个服务"，得给出照着做的说法。
  it("points at the installable service when the backend says so", async () => {
    await mount({ windows: [], probeHint: true });
    expect(document.querySelector(".overlay-hint")?.textContent).toBe(
      "Screenshots work better with a small GNOME service: install it in Settings \u2192 Screenshot, "
        + "then log out once. Drag to select an area in the meantime.",
    );
  });

  // 后端只在首次遇到时置 probeHint，此后即使还是没装也不再提示——不装照样能框选。
  it("falls back to the plain notice once the one-time hint has been spent", async () => {
    await mount({ windows: [], probeHint: false });
    expect(document.querySelector(".overlay-hint")?.textContent)
      .toBe("Window picking unavailable in this session — drag to select an area");
  });

  // 覆盖层是隐藏建窗的，显示时机由前端决定：早一步显示就是一整屏白屏。
  it("asks the backend to reveal the window only after the first frame is drawn", async () => {
    let deliverPayload;
    mocks.overlayApi.get.mockReturnValue(new Promise((resolve) => (deliverPayload = resolve)));
    await act(async () => root.render(React.createElement(App)));
    // payload 还没到，冻结帧也没画：这时候显示出来就是一整屏白色
    expect(mocks.overlayApi.ready).not.toHaveBeenCalled();

    await act(async () => deliverPayload(basePayload));
    await flush();
    expect(mocks.overlayApi.ready).toHaveBeenCalledWith("capture-overlay-session-1-0");
  });

  it("reveals the window once, not on every redraw", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });
    await drag({ x: 20, y: 20 }, { x: 60, y: 50 });

    expect(mocks.overlayApi.ready).toHaveBeenCalledTimes(1);
  });

  it("reveals the window to show a failure instead of staying invisible", async () => {
    mocks.overlayApi.get.mockRejectedValue(new Error("frame gone"));
    await act(async () => root.render(React.createElement(App)));
    await flush();

    expect(mocks.overlayApi.ready).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".overlay-error")?.textContent).toContain("frame gone");
  });

  /**
   * 像素走二进制 IPC，尺寸只由 payload 声明，没有 PNG 头去自我校验。
   * 字节数不对就必须报错并把窗口显示出来，绝不能拿错位的像素当底图铺满全屏。
   */
  it("reports a truncated frame buffer instead of drawing skewed pixels", async () => {
    mocks.overlayApi.get.mockResolvedValue(basePayload);
    mocks.overlayApi.image.mockRejectedValue(new Error("protocol unavailable"));
    mocks.overlayApi.frame.mockResolvedValue(new ArrayBuffer(64));
    await act(async () => root.render(React.createElement(App)));
    await flush();

    expect(mocks.overlayApi.frame).toHaveBeenCalledWith("capture-overlay-session-1-0");
    expect(mocks.overlayApi.ready).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".overlay-error")).not.toBeNull();
  });

  it("does not block capture when revealing fails", async () => {
    mocks.overlayApi.ready.mockRejectedValue(new Error("no window"));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    expect(document.querySelector(".overlay-toolbar")).not.toBeNull();
    expect(document.querySelector(".overlay-error")).toBeNull();
  });
});
