import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  allowsTextSelection,
  isZoomShortcut,
  pinWheelIntent,
} from "../react/pin/gestures.ts";

const mocks = vi.hoisted(() => ({
  getCurrentWindowLabel: vi.fn(() => "pin-image-test"),
  startDraggingCurrentWindow: vi.fn(),
  pinApi: {
    get: vi.fn(),
    ready: vi.fn(),
    update: vi.fn(),
    copy: vi.fn(),
    save: vi.fn(),
    close: vi.fn(),
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
  canSave: true,
  position: null,
};

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("pin wheel and pinch rules", () => {
  it("plain wheel scales the pin", () => {
    expect(pinWheelIntent({ deltaY: -3, ctrlKey: false, metaKey: false, shiftKey: false }))
      .toEqual({ kind: "scale", delta: 0.05 });
    expect(pinWheelIntent({ deltaY: 3, ctrlKey: false, metaKey: false, shiftKey: false }))
      .toEqual({ kind: "scale", delta: -0.05 });
  });

  it("shift wheel changes opacity so pinch cannot hit it by accident", () => {
    expect(pinWheelIntent({ deltaY: -3, ctrlKey: false, metaKey: false, shiftKey: true }))
      .toEqual({ kind: "opacity", delta: 0.05 });
  });

  /**
   * WebKitGTK 把触控板捏合合成成 ctrl+滚轮。贴图窗口按设计不可缩放，
   * 所以这一路必须什么都不做——既不缩放窗口，也不能改不透明度。
   */
  it("ctrl or meta wheel does nothing at all", () => {
    for (const modifier of [{ ctrlKey: true, metaKey: false }, { ctrlKey: false, metaKey: true }]) {
      expect(pinWheelIntent({ deltaY: -3, shiftKey: false, ...modifier }))
        .toEqual({ kind: "ignore" });
      expect(pinWheelIntent({ deltaY: 3, shiftKey: true, ...modifier }))
        .toEqual({ kind: "ignore" });
    }
  });

  it("recognizes WebKit page-zoom shortcuts", () => {
    for (const key of ["+", "-", "=", "_", "0"]) {
      expect(isZoomShortcut({ key, ctrlKey: true, metaKey: false })).toBe(true);
    }
    expect(isZoomShortcut({ key: "+", ctrlKey: false, metaKey: false })).toBe(false);
    expect(isZoomShortcut({ key: "c", ctrlKey: true, metaKey: false })).toBe(false);
  });

  it("allows selecting text pins but nothing else", () => {
    document.body.innerHTML = `
      <main class="pin-root">
        <section class="pin-media image"><img id="image" alt=""></section>
        <section class="pin-media text"><pre id="text">hello</pre></section>
        <input id="field">
      </main>`;
    expect(allowsTextSelection(document.getElementById("text"))).toBe(true);
    expect(allowsTextSelection(document.getElementById("field"))).toBe(true);
    expect(allowsTextSelection(document.getElementById("image"))).toBe(false);
    expect(allowsTextSelection(document.querySelector(".pin-root"))).toBe(false);
    expect(allowsTextSelection(null)).toBe(false);
  });
});

describe("pin window is not zoomable and not selectable", () => {
  let root;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    i18n.init("en");
    for (const fn of Object.values(mocks.pinApi)) fn.mockReset();
    mocks.pinApi.ready.mockResolvedValue(undefined);
    mocks.pinApi.close.mockResolvedValue(undefined);
    mocks.pinApi.get.mockResolvedValue(payload);
    mocks.pinApi.update.mockImplementation(async (_label, update) => ({ ...payload, ...update }));
    root = createRoot(document.getElementById("root"));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  /**
   * 回归防线：React 的 `onWheel` 是被动监听器，在里面 `preventDefault()` 是空操作，
   * 于是 ctrl+滚轮（也就是触控板捏合）会落到 WebKit 的页面缩放上。所以必须是
   * 自己用 `{ passive: false }` 注册的原生监听器。
   */
  it("cancels ctrl+wheel instead of letting WebKit zoom the page", async () => {
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const zoomed = new WheelEvent("wheel", {
      deltaY: -120,
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    await act(async () => {
      document.querySelector(".pin-root").dispatchEvent(zoomed);
    });

    expect(zoomed.defaultPrevented).toBe(true);
    // 什么都不该改：既不缩放，也不调不透明度
    expect(document.querySelector(".pin-scale")?.textContent).toBe("100");
    expect(mocks.pinApi.update).not.toHaveBeenCalled();
  });

  it("still scales on a plain wheel", async () => {
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const scrolled = new WheelEvent("wheel", {
      deltaY: -120,
      bubbles: true,
      cancelable: true,
    });
    await act(async () => {
      document.querySelector(".pin-root").dispatchEvent(scrolled);
    });

    expect(scrolled.defaultPrevented).toBe(true);
    expect(document.querySelector(".pin-scale")?.textContent).toBe("105");
  });

  /** 拖动贴图不能变成"选中内容"——那会把整块刷成系统强调色。 */
  it("cancels selection and native drags outside text pins", async () => {
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const onRoot = new Event("selectstart", { bubbles: true, cancelable: true });
    await act(async () => document.querySelector(".pin-root").dispatchEvent(onRoot));
    expect(onRoot.defaultPrevented).toBe(true);

    // 文本贴图里的 <pre> 仍然可以划选，否则文本贴图就没用了
    const onText = new Event("selectstart", { bubbles: true, cancelable: true });
    await act(async () => document.querySelector(".pin-media pre").dispatchEvent(onText));
    expect(onText.defaultPrevented).toBe(false);

    const drag = new Event("dragstart", { bubbles: true, cancelable: true });
    await act(async () => document.querySelector(".pin-root").dispatchEvent(drag));
    expect(drag.defaultPrevented).toBe(true);
  });

  /**
   * `update_pin` 的应答只带可变字段（后端每帧重编一张 base64 图片纯属浪费），
   * 前端必须把它合并进手里的 payload——直接整份替换会把内容和 canSave 抹成 undefined。
   */
  it("merges the image-free update response into the existing payload", async () => {
    mocks.pinApi.update.mockImplementation(async (label, update) => ({
      label,
      contentWidth: payload.contentWidth,
      contentHeight: payload.contentHeight,
      scale: update.scale ?? payload.scale,
      opacity: update.opacity ?? payload.opacity,
      locked: update.locked ?? payload.locked,
      position: null,
    }));
    await act(async () => root.render(React.createElement(App)));
    await flush();

    await act(async () => {
      document.querySelector(".pin-root").dispatchEvent(
        new WheelEvent("wheel", { deltaY: -120, bubbles: true, cancelable: true }),
      );
    });
    // 更新按 rAF 合并，等它真的发出去
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 24));
    });

    expect(mocks.pinApi.update).toHaveBeenCalledWith("pin-image-test", { scale: 1.05 });
    expect(document.querySelector(".pin-scale")?.textContent).toBe("105");
    // 内容与"能不能保存"都来自本地那份 payload，不能被应答抹掉
    expect(document.querySelector(".pin-media pre")?.textContent).toBe("Pinned text");
    expect(document.querySelector('[aria-label="Save image"]')).not.toBeNull();
  });

  /** 键盘的页面缩放快捷键也要吃掉，并且转成贴图自己的缩放。 */
  it("turns ctrl+plus into pin scaling rather than page zoom", async () => {
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const event = new KeyboardEvent("keydown", {
      key: "=",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    await act(async () => window.dispatchEvent(event));

    expect(event.defaultPrevented).toBe(true);
    expect(document.querySelector(".pin-scale")?.textContent).toBe("110");
  });
});
