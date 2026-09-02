import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  allowsTextSelection,
  isZoomShortcut,
  NO_DRAG,
  pinWheelIntent,
  trackDragMove,
  trackDragPointerDown,
} from "../react/pin/gestures.ts";

const mocks = vi.hoisted(() => ({
  getCurrentWindowLabel: vi.fn(() => "pin-image-test"),
  startDraggingCurrentWindow: vi.fn(),
  pinApi: {
    get: vi.fn(),
    platform: vi.fn(),
    ready: vi.fn(),
    update: vi.fn(),
    copy: vi.fn(),
    save: vi.fn(),
    close: vi.fn(),
    onSharpened: vi.fn(),
    onAlreadyOpen: vi.fn(),
    toolbarBounds: vi.fn(),
    sourceImage: vi.fn(),
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
  deviceScale: 1,
  bufferScale: 1,
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

/**
 * 拖动判据的单元测试。回归的是那个"第一下能拖、第二下拖不动、第三下又能拖"的毛病。
 *
 * 根因不是某一个迟到的事件，而是"拖动依赖跨事件记账"这件事本身：Wayland 上
 * `startDragging` 之后指针被合成器抓走，这一次的 `pointerup` 永远送不到 WebKit，
 * 于是每次拖完都会留下一件迟到的事情落在**下一次**按压之后——`pointercancel`、
 * `buttons=0` 的收尾 `pointermove`、或者 WebKit 那份"按键还按着"的残留状态把下一个
 * `pointerdown` 整个吃掉。三种机制症状一样，也都会污染任何记账。
 *
 * 所以这里锁死的规则是：**每个事件只按自己带的数据判断**，而且起点可以从
 * `pointermove` 自己长出来，一个 `pointerdown` 都不需要。
 */
describe("drag tracking judges each event on its own data", () => {
  const held = { buttons: 1, onControls: false };

  /** 核心一条：`pointerdown` 被吞掉时，光靠 `pointermove` 也必须能拖起来。 */
  it("starts a drag from pointermove alone, with no pointerdown at all", () => {
    const armed = trackDragMove(NO_DRAG, { ...held, x: 40, y: 40 }, 1_000);
    expect(armed.start).toBe(false);
    expect(armed.state.origin).toEqual({ x: 40, y: 40 });

    const moved = trackDragMove(armed.state, { ...held, x: 60, y: 60 }, 1_010);
    expect(moved.start).toBe(true);
    // 起点用完即清：合成器接手后 WebKit 收不到后续事件
    expect(moved.state.origin).toBeNull();
  });

  it("needs more than a few pixels so a plain click never moves the window", () => {
    const armed = trackDragMove(NO_DRAG, { ...held, x: 40, y: 40 }, 0);
    expect(trackDragMove(armed.state, { ...held, x: 43, y: 40 }, 5).start).toBe(false);
    expect(trackDragMove(armed.state, { ...held, x: 46, y: 40 }, 5).start).toBe(true);
  });

  /** 松手把一切忘掉——包括"从工具条按下去"的抑制，不然抑制会漏到下一次按压。 */
  it("forgets everything once the button is released", () => {
    const suppressed = trackDragPointerDown(NO_DRAG, {
      button: 0,
      x: 10,
      y: 10,
      onControls: true,
    });
    expect(suppressed.suppressed).toBe(true);

    const released = trackDragMove(suppressed, { buttons: 0, x: 90, y: 90, onControls: false }, 20);
    expect(released.start).toBe(false);
    expect(released.state).toEqual(NO_DRAG);
  });

  /** 从工具条按下去的那次按压，全程都不许拖窗口（滑块得能拖到底）。 */
  it("keeps a toolbar press suppressed for the whole gesture", () => {
    let state = trackDragPointerDown(NO_DRAG, { button: 0, x: 10, y: 10, onControls: true });
    for (const x of [40, 80, 160]) {
      const step = trackDragMove(state, { ...held, x, y: 10 }, x);
      expect(step.start).toBe(false);
      state = step.state;
    }
  });

  /** 起点还没长出来时落在工具条上的移动不算起点，否则会从滑块上把窗口拖走。 */
  it("does not arm an origin from a move over the toolbar", () => {
    const onControls = trackDragMove(NO_DRAG, { buttons: 1, x: 10, y: 10, onControls: true }, 0);
    expect(onControls.state.origin).toBeNull();
    expect(onControls.start).toBe(false);
  });

  it("ignores presses that are not the primary button", () => {
    expect(trackDragPointerDown(NO_DRAG, { button: 2, x: 5, y: 5, onControls: false }))
      .toEqual(NO_DRAG);
    expect(trackDragMove(NO_DRAG, { buttons: 2, x: 5, y: 5, onControls: false }, 0).state)
      .toEqual(NO_DRAG);
  });

  /**
   * 限速只防"合成器没接手指针、`pointermove` 继续送进来"那一路：没有新按压时
   * 300 ms 内不重复发。一个真的新按压带着 `pointerdown` 会把它清零，
   * 所以用户明确发起的下一次拖动**永远不会**被冷却挡住——那就是原来那个 bug。
   */
  it("rate limits only the retry path, never a fresh press", () => {
    const first = trackDragMove(
      trackDragMove(NO_DRAG, { ...held, x: 40, y: 40 }, 1_000).state,
      { ...held, x: 60, y: 60 },
      1_000,
    );
    expect(first.start).toBe(true);

    // 没有 pointerdown，紧接着又移动：限速挡住
    const throttled = trackDragMove(
      trackDragMove(first.state, { ...held, x: 60, y: 60 }, 1_050).state,
      { ...held, x: 90, y: 90 },
      1_060,
    );
    expect(throttled.start).toBe(false);
    // 过了冷却就该放行
    const retried = trackDragMove(
      trackDragMove(first.state, { ...held, x: 60, y: 60 }, 1_500).state,
      { ...held, x: 90, y: 90 },
      1_510,
    );
    expect(retried.start).toBe(true);

    // 而带着 pointerdown 的新按压立刻就能拖，哪怕只过了 10 ms
    const pressed = trackDragPointerDown(first.state, {
      button: 0,
      x: 60,
      y: 60,
      onControls: false,
    });
    expect(trackDragMove(pressed, { ...held, x: 90, y: 90 }, 1_010).start).toBe(true);
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
    mocks.pinApi.platform.mockResolvedValue({
      capabilities: { always_on_top: { state: "available", reason: null } },
    });
    mocks.pinApi.onSharpened.mockResolvedValue(() => {});
    mocks.pinApi.onAlreadyOpen.mockResolvedValue(() => {});
    mocks.pinApi.toolbarBounds.mockResolvedValue({ x: 0, y: 0, width: 388, height: 252 });
    mocks.pinApi.sourceImage.mockResolvedValue(null);
    mocks.startDraggingCurrentWindow.mockReset();
    mocks.startDraggingCurrentWindow.mockResolvedValue(undefined);
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

  /**
   * 回归防线："第一下能拖、第二下拖不动、第三下又能拖"。
   *
   * Wayland 上 `startDragging` 之后指针被合成器抓走，这一次的 `pointerup` 不会送到
   * WebKit，迟到的 `pointercancel` 往往落在**下一次** `pointerdown` 之后。所以拖动判据
   * 不能依赖跨事件记账，只能看每个事件自带的按键状态。
   */
  it("keeps dragging on every gesture even when pointercancel arrives late", async () => {
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const root_ = document.querySelector(".pin-root");
    // jsdom 没有 PointerEvent 构造器，用 MouseEvent 派发同名事件；React 只看类型名。
    const pointer = (type, init) =>
      new MouseEvent(type, { bubbles: true, cancelable: true, ...init });

    async function dragOnce() {
      await act(async () => root_.dispatchEvent(pointer("pointerdown", { button: 0, buttons: 1, clientX: 40, clientY: 40 })));
      await act(async () => window.dispatchEvent(pointer("pointermove", { buttons: 1, clientX: 60, clientY: 60 })));
    }

    await dragOnce();
    expect(mocks.startDraggingCurrentWindow).toHaveBeenCalledTimes(1);

    // 合成器抓走指针后迟到的取消事件：它现在什么都不该影响
    await act(async () => window.dispatchEvent(pointer("pointercancel", { buttons: 0 })));

    await dragOnce();
    expect(mocks.startDraggingCurrentWindow).toHaveBeenCalledTimes(2);

    // 松手后的移动依然不算拖动
    await act(async () => window.dispatchEvent(pointer("pointermove", { buttons: 0, clientX: 200, clientY: 200 })));
    expect(mocks.startDraggingCurrentWindow).toHaveBeenCalledTimes(2);
  });

  /**
   * 同一个 bug 的第三种机制：WebKit 那份"按键还按着"的残留状态把下一次的
   * `pointerdown` 整个吃掉。这时候只剩 `pointermove`，也必须能拖起来。
   */
  it("drags from pointermove alone when the pointerdown never arrives", async () => {
    await act(async () => root.render(React.createElement(App)));
    await flush();

    const pointer = (type, init) =>
      new MouseEvent(type, { bubbles: true, cancelable: true, ...init });
    // 一个 pointerdown 都不发，直接按住移动
    await act(async () =>
      window.dispatchEvent(pointer("pointermove", { buttons: 1, clientX: 40, clientY: 40 })));
    expect(mocks.startDraggingCurrentWindow).not.toHaveBeenCalled();

    await act(async () =>
      window.dispatchEvent(pointer("pointermove", { buttons: 1, clientX: 70, clientY: 70 })));
    expect(mocks.startDraggingCurrentWindow).toHaveBeenCalledTimes(1);
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
