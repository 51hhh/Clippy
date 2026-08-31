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
});
