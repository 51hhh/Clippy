import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getCurrentWindowLabel: vi.fn(() => "capture-overlay-session-1-0"),
  getConfig: vi.fn(async () => ({ language: "en" })),
  overlayApi: {
    get: vi.fn(),
    cancel: vi.fn(),
    run: vi.fn(),
    translate: vi.fn(),
    copyText: vi.fn(),
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
  pngBase64: "AAEC",
  logicalWidth: 200,
  logicalHeight: 150,
  pixelWidth: 200,
  pixelHeight: 150,
  windows: [{ x: 40, y: 30, width: 60, height: 50, title: "editor" }],
  commitAction: "editor",
};

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function pointer(type, x, y, altKey = false) {
  const root = document.querySelector(".overlay-root");
  root.dispatchEvent(
    new MouseEvent(type, { bubbles: true, clientX: x, clientY: y, button: 0, altKey }),
  );
}

async function drag(from, to, { altKey = false } = {}) {
  await act(async () => {
    pointer("pointerdown", from.x, from.y);
    pointer("pointermove", to.x, to.y);
    pointer("pointerup", to.x, to.y, altKey);
  });
  await flush();
}

describe("capture overlay app", () => {
  let root;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    i18n.init("en");
    // jsdom 没有这两样，覆盖层的指针捕获与 Blob URL 都会直接抛错。
    Element.prototype.setPointerCapture = () => {};
    URL.createObjectURL = vi.fn(() => "blob:capture");
    URL.revokeObjectURL = vi.fn();
    for (const fn of Object.values(mocks.overlayApi)) fn.mockReset();
    mocks.overlayApi.run.mockResolvedValue({ action: "edit", path: null, pinLabel: null });
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

  it("opens the editor as soon as a selection is committed", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    expect(mocks.overlayApi.run).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.run).toHaveBeenCalledWith("edit", {
      x: 10,
      y: 10,
      width: 90,
      height: 70,
      sessionId: "session-1",
      monitorId: 0,
    });
  });

  it("opens the editor for a clicked window without dragging", async () => {
    await mount();
    await drag({ x: 50, y: 40 }, { x: 51, y: 40 });

    expect(mocks.overlayApi.run).toHaveBeenCalledWith("edit", {
      x: 40,
      y: 30,
      width: 60,
      height: 50,
      sessionId: "session-1",
      monitorId: 0,
    });
  });

  it("keeps the toolbar when Alt is held on release", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 }, { altKey: true });

    // 默认直开编辑器，但选区翻译必须还有入口
    expect(mocks.overlayApi.run).not.toHaveBeenCalled();
    expect(document.querySelector('button[aria-label="Translate selection"]')).not.toBeNull();
  });

  it("stops on the toolbar when the setting says so", async () => {
    await mount({ commitAction: "toolbar" });
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    expect(mocks.overlayApi.run).not.toHaveBeenCalled();
    expect(document.querySelector(".overlay-toolbar")).not.toBeNull();
    // 工具条上的编辑按钮仍然走同一条动作出口
    await act(async () => document.querySelector('button[aria-label="Edit"]').click());
    await flush();
    expect(mocks.overlayApi.run).toHaveBeenCalledWith("edit", expect.objectContaining({ width: 90 }));
  });

  it("does not commit a click on empty space", async () => {
    await mount({ windows: [] });
    await drag({ x: 150, y: 120 }, { x: 151, y: 120 });

    expect(mocks.overlayApi.run).not.toHaveBeenCalled();
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
