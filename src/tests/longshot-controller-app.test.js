import React, { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getConfig: vi.fn(async () => ({ language: "en" })),
  controllerApi: {
    activate: vi.fn(),
    append: vi.fn(),
    ready: vi.fn(),
    cancel: vi.fn(),
    onCloseRequested: vi.fn(),
  },
}));

vi.mock("../js/api.ts", () => ({
  getConfig: mocks.getConfig,
}));
vi.mock("../react/longshot-controller/api.ts", () => ({
  longshotControllerApi: mocks.controllerApi,
}));

import * as i18n from "../i18n/i18n.js";
import { App } from "../react/longshot-controller/App.tsx";

const activation = {
  handle: { sessionId: "longshot-7", generation: "18446744073709551615" },
  snapshot: { frameCount: 3, width: 900, frameHeight: 600, totalHeight: 1700 },
};

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("longshot controller app", () => {
  let root;
  let closeRequested;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    i18n.init("en");
    closeRequested = undefined;
    for (const fn of Object.values(mocks.controllerApi)) fn.mockReset();
    mocks.controllerApi.activate.mockResolvedValue(activation);
    mocks.controllerApi.append.mockResolvedValue(activation.snapshot);
    mocks.controllerApi.ready.mockResolvedValue(undefined);
    mocks.controllerApi.cancel.mockResolvedValue(undefined);
    mocks.controllerApi.onCloseRequested.mockImplementation((callback) => {
      closeRequested = callback;
      return Promise.resolve(vi.fn());
    });
    root = createRoot(document.getElementById("root"));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  async function mount(strict = true) {
    await act(async () => {
      root.render(strict
        ? React.createElement(StrictMode, null, React.createElement(App))
        : React.createElement(App));
    });
    await flush();
  }

  it("activates exactly once under StrictMode, then reveals the rendered snapshot", async () => {
    mocks.controllerApi.ready.mockImplementation(() => {
      expect(document.body.textContent).toContain("1700");
      return Promise.resolve();
    });
    await mount();

    expect(mocks.controllerApi.activate).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.ready).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".longshot-titlebar")?.hasAttribute("data-tauri-drag-region"))
      .toBe(true);
    expect(document.querySelector(".longshot-titlebar h1")?.hasAttribute("data-tauri-drag-region"))
      .toBe(true);
    expect(document.querySelector("button")?.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(document.body.textContent).toContain("Frames");
    expect(document.body.textContent).toContain("3");
    expect(document.body.textContent).toContain("1700");
  });

  it("renders a structured activation failure, reveals it, and closes with a null handle", async () => {
    mocks.controllerApi.activate.mockRejectedValue({
      code: "longshot_controller_busy",
      message: "busy",
    });
    await mount();

    expect(mocks.controllerApi.ready).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("already active");
    await act(async () => document.querySelector("button").click());

    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(null);
  });

  it("passes the complete string handle exactly once for Cancel and Escape", async () => {
    await mount();
    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flush();

    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(activation.handle);
    expect(document.body.textContent).toContain("Cancelling");
  });

  it("uses the same cancel path for the native close request", async () => {
    await mount();
    expect(closeRequested).toEqual(expect.any(Function));

    await act(async () => closeRequested());
    await flush();

    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(activation.handle);
  });

  it("updates the rendered snapshot after Append success with the complete string handle", async () => {
    const nextSnapshot = {
      frameCount: 4,
      width: 900,
      frameHeight: 600,
      totalHeight: 2210,
    };
    mocks.controllerApi.append.mockResolvedValue(nextSnapshot);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await flush();

    expect(mocks.controllerApi.append).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.append).toHaveBeenCalledWith({
      sessionId: "longshot-7",
      generation: "18446744073709551615",
    });
    expect(document.body.textContent).toContain("4");
    expect(document.body.textContent).toContain("2210");
    expect(document.body.textContent).toContain("Ready to capture more frames.");
  });

  it("keeps the previous snapshot after an append business error and allows retry", async () => {
    const nextSnapshot = {
      frameCount: 4,
      width: 900,
      frameHeight: 600,
      totalHeight: 2210,
    };
    mocks.controllerApi.append
      .mockRejectedValueOnce({ code: "longshot_estimate_low_texture", message: "untrusted worker detail" })
      .mockResolvedValueOnce(nextSnapshot);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await flush();

    expect(document.body.textContent).toContain("3");
    expect(document.body.textContent).toContain("1700");
    expect(document.body.textContent).toContain("Could not append another frame");
    expect(document.body.textContent).not.toContain("untrusted worker detail");

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await flush();
    expect(mocks.controllerApi.append).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).toContain("2210");
  });

  it("prevents a double click from starting a second Append attempt", async () => {
    const pending = deferred();
    mocks.controllerApi.append.mockReturnValue(pending.promise);
    await mount();

    await act(async () => {
      document.querySelector('[data-testid="longshot-append"]').click();
      document.querySelector('[data-testid="longshot-append"]').click();
    });

    expect(mocks.controllerApi.append).toHaveBeenCalledTimes(1);
    expect(document.querySelector('[data-testid="longshot-append"]')?.disabled).toBe(true);
    expect(document.body.textContent).toContain("Capturing another frame");

    await act(async () => pending.resolve(activation.snapshot));
    await flush();
  });

  it.each([
    ["Cancel", async () => document.querySelector('[data-testid="longshot-cancel"]').click()],
    ["Escape", async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))],
    ["native close", async () => closeRequested()],
  ])("cancels exactly once when %s occurs during Append and ignores a late resolution", async (_name, trigger) => {
    const pending = deferred();
    mocks.controllerApi.append.mockReturnValue(pending.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await act(async () => trigger());
    await flush();
    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(activation.handle);
    expect(document.body.textContent).toContain("Cancelling");

    await act(async () => pending.resolve({ ...activation.snapshot, frameCount: 99, totalHeight: 9999 }));
    await flush();
    expect(document.body.textContent).toContain("Cancelling");
    expect(document.body.textContent).not.toContain("9999");
  });

  it("ignores a late append rejection after cancellation has won", async () => {
    const pending = deferred();
    mocks.controllerApi.append.mockReturnValue(pending.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    await act(async () => pending.reject({ code: "longshot_controller_internal", message: "late detail" }));
    await flush();

    expect(document.body.textContent).toContain("Cancelling");
    expect(document.body.textContent).not.toContain("late detail");
  });

  it("keeps restart guidance when a cancel cleanup failure beats a late Append result", async () => {
    const pending = deferred();
    mocks.controllerApi.append.mockReturnValue(pending.promise);
    mocks.controllerApi.cancel.mockRejectedValue({
      code: "longshot_controller_cleanup_failed",
      message: "untrusted cancellation detail",
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    await flush();
    expect(document.body.textContent).toContain("Restart Clippy");

    await act(async () => pending.resolve({ ...activation.snapshot, frameCount: 99, totalHeight: 9999 }));
    await flush();
    expect(document.body.textContent).toContain("Restart Clippy");
    expect(document.body.textContent).not.toContain("9999");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
  });

  it("invalidates a pending Append attempt when the controller unmounts", async () => {
    const pending = deferred();
    mocks.controllerApi.append.mockReturnValue(pending.promise);
    await mount(false);

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await act(async () => root.unmount());
    await act(async () => pending.resolve({ ...activation.snapshot, frameCount: 99, totalHeight: 9999 }));
    await flush();

    expect(document.body.textContent).not.toContain("9999");
  });

  it("ignores a late ready rejection after the controller unmounts", async () => {
    const pending = deferred();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.controllerApi.ready.mockReturnValue(pending.promise);
    await mount(false);

    await act(async () => root.unmount());
    await act(async () => pending.reject({ code: "longshot_controller_internal", message: "late ready" }));
    await flush();

    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  it("shows restart guidance without retry actions after an append cleanup failure", async () => {
    mocks.controllerApi.append.mockRejectedValue({
      code: "longshot_controller_cleanup_failed",
      message: "untrusted cleanup detail",
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await flush();

    expect(document.body.textContent).toContain("Restart Clippy");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-cancel"]')).toBeNull();
    expect(document.body.textContent).not.toContain("untrusted cleanup detail");
  });

  it.each(["longshot_controller_hide_failed", "longshot_controller_internal"])(
    "localizes the retryable %s append error without needing a backend message",
    async (code) => {
      mocks.controllerApi.append.mockRejectedValue({ code });
      await mount();

      await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
      await flush();

      expect(document.body.textContent).toContain("Could not append another frame");
    },
  );

  it("keeps the handle and returns to Ready after a retryable cancellation failure", async () => {
    mocks.controllerApi.cancel
      .mockRejectedValueOnce({ code: "longshot_controller_internal", message: "temporary" })
      .mockResolvedValueOnce(undefined);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    await flush();
    expect(document.body.textContent).toContain("Could not start");
    expect(document.body.textContent).toContain("Cancel");

    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    expect(mocks.controllerApi.cancel).toHaveBeenNthCalledWith(1, activation.handle);
    expect(mocks.controllerApi.cancel).toHaveBeenNthCalledWith(2, activation.handle);
  });

  it("keeps a safe cancellation fallback during a ready teardown race", async () => {
    mocks.controllerApi.ready.mockRejectedValue({
      code: "longshot_controller_missing",
      message: "show teardown is in flight",
    });
    await mount();

    // ready 的拒绝不能证明原生窗口/后端 cleanup 已经完成。前端不把它当作可重试
    // Active，只在窗口尚存的短暂竞态中保留 handle 作为 best-effort 关闭兜底。
    expect(document.body.textContent).toContain("Could not start");
    expect(document.querySelector('[data-testid="longshot-cancel"]')?.textContent).toBe("Cancel");
    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());

    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(activation.handle);
  });

  it("shows restart guidance for a non-retryable cleanup failure", async () => {
    mocks.controllerApi.cancel.mockRejectedValue({
      code: "longshot_controller_cleanup_failed",
      message: "unknown desktop state",
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    await flush();

    expect(document.body.textContent).toContain("Restart Clippy");
    expect(document.querySelector("button")).toBeNull();
  });

  it("reveals a cleanup failure only after its restart guidance has rendered", async () => {
    mocks.controllerApi.activate.mockRejectedValue({
      code: "longshot_controller_cleanup_failed",
      message: "join cleanup could not be confirmed",
    });
    mocks.controllerApi.ready.mockImplementation(() => {
      expect(document.body.textContent).toContain("Restart Clippy");
      expect(document.querySelector("button")).toBeNull();
      return Promise.resolve();
    });

    await mount();

    expect(mocks.controllerApi.ready).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("Restart Clippy");
    expect(document.querySelector("button")).toBeNull();
  });

  it("handles malformed thrown values without displaying untrusted error text", async () => {
    mocks.controllerApi.activate.mockRejectedValue("untrusted backend message");
    await mount();

    expect(document.body.textContent).toContain("Could not start");
    expect(document.body.textContent).not.toContain("untrusted backend message");
  });
});
