import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getCurrentWindowLabel: vi.fn(() => "capture-overlay-session-1-0"),
  getConfig: vi.fn(async () => ({ language: "en" })),
  overlayApi: {
    get: vi.fn(),
    cancel: vi.fn(),
    commit: vi.fn(),
    translate: vi.fn(),
    copyText: vi.fn(),
  },
  exportPngBase64: vi.fn(async () => "exported-png"),
  pngBase64ToObjectUrl: vi.fn(() => "blob:capture"),
}));

vi.mock("../js/api.ts", () => ({
  getCurrentWindowLabel: mocks.getCurrentWindowLabel,
  getConfig: mocks.getConfig,
}));
vi.mock("../react/capture-overlay/api.ts", () => ({ overlayApi: mocks.overlayApi }));
// jsdom 没有 canvas 后端，导出必须替身；同时记录裁剪矩形，用来断言"裁剪 + 标注"合成。
vi.mock("../react/annotation/pngPipeline", () => ({
  exportPngBase64: mocks.exportPngBase64,
  pngBase64ToObjectUrl: mocks.pngBase64ToObjectUrl,
}));

import * as i18n from "../i18n/i18n.js";
import { App } from "../react/capture-overlay/App.tsx";

const basePayload = {
  sessionId: "session-1",
  monitorId: 0,
  pngBase64: "AAEC",
  logicalWidth: 200,
  logicalHeight: 150,
  pixelWidth: 200,
  pixelHeight: 150,
  windows: [{ x: 40, y: 30, width: 60, height: 50, title: "editor" }],
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

describe("capture overlay app", () => {
  let root;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    i18n.init("en");
    // jsdom 缺这些：覆盖层的指针捕获、Blob URL 与画布上下文都会直接抛错。
    Element.prototype.setPointerCapture = () => {};
    HTMLCanvasElement.prototype.getContext = () => null;
    URL.createObjectURL = vi.fn(() => "blob:capture");
    URL.revokeObjectURL = vi.fn();
    // jsdom 不会真的去解码 blob: URL，onload 永远不触发，冻结帧就一直"没准备好"。
    globalThis.Image = class {
      naturalWidth = basePayload.pixelWidth;
      naturalHeight = basePayload.pixelHeight;
      onload = null;
      onerror = null;
      set src(value) {
        this._src = value;
        queueMicrotask(() => this.onload?.());
      }
      get src() {
        return this._src;
      }
    };
    for (const fn of Object.values(mocks.overlayApi)) fn.mockReset();
    mocks.exportPngBase64.mockClear();
    mocks.overlayApi.commit.mockResolvedValue({ action: "copy", path: null, pinLabel: null });
    root = createRoot(document.getElementById("root"));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  async function mount(overrides = {}) {
    mocks.overlayApi.get.mockResolvedValue({ ...basePayload, ...overrides });
    await act(async () => root.render(React.createElement(App)));
    await flush();
  }

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

  it("copies the cropped and annotated PNG when the check mark is pressed", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Copy").click());
    await flush();

    // 裁剪在前端完成：后端只落地这张 PNG，否则画布上的标注会被丢掉
    expect(mocks.exportPngBase64).toHaveBeenCalledTimes(1);
    expect(mocks.exportPngBase64.mock.calls[0][1]).toEqual({
      x: 10,
      y: 10,
      width: 100,
      height: 80,
    });
    expect(mocks.overlayApi.commit).toHaveBeenCalledWith("copy", "session-1", "exported-png");
  });

  it("routes save and pin through the same commit path", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => button("Pin").click());
    await flush();
    expect(mocks.overlayApi.commit).toHaveBeenCalledWith("pin", "session-1", "exported-png");
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
});
