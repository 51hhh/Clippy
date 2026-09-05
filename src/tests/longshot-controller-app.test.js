import React, { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getConfig: vi.fn(async () => ({ language: "en" })),
  controllerApi: {
    activate: vi.fn(),
    append: vi.fn(),
    finish: vi.fn(),
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
    mocks.controllerApi.finish.mockResolvedValue({ action: "copy", path: null });
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

  it("finishes with Copy exactly once and keeps the result terminal while the backend closes", async () => {
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.finish).toHaveBeenCalledWith(activation.handle, "copy");
    expect(document.body.textContent).toContain("Copied. Closing this window");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-copy"]')).toBeNull();
  });

  it("finishes with Save exactly once and renders the returned Unicode path as text", async () => {
    mocks.controllerApi.finish.mockResolvedValue({ action: "save", path: "/tmp/长截图/结果.png" });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.finish).toHaveBeenCalledWith(activation.handle, "save");
    expect(document.body.textContent).toContain("Saved to /tmp/长截图/结果.png. Closing this window");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-save"]')).toBeNull();
  });

  it("keeps the snapshot and Ready controls after a retryable finish-domain failure", async () => {
    mocks.controllerApi.finish.mockRejectedValue({
      code: "longshot_estimate_low_texture",
      message: "untrusted encoder detail",
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();

    expect(document.body.textContent).toContain("3");
    expect(document.body.textContent).toContain("1700");
    expect(document.body.textContent).toContain("Could not finish the long screenshot");
    expect(document.body.textContent).not.toContain("untrusted encoder detail");
    expect(document.querySelector('[data-testid="longshot-append"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-copy"]')).not.toBeNull();
  });

  it("prevents same-tick Copy/Save reentry and disables every output action while Finishing", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount();

    await act(async () => {
      document.querySelector('[data-testid="longshot-copy"]').click();
      document.querySelector('[data-testid="longshot-save"]').click();
    });

    expect(mocks.controllerApi.finish).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.finish).toHaveBeenCalledWith(activation.handle, "copy");
    expect(document.querySelector('[data-testid="longshot-copy"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-testid="longshot-save"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-testid="longshot-append"]')?.disabled).toBe(true);
    expect(document.body.textContent).toContain("Finishing the long screenshot and copying it");

    await act(async () => pending.resolve({ action: "copy", path: null }));
    await flush();
  });

  it("moves a Copy failure to Any OutputPending and can retry it as Save without reopening Append", async () => {
    mocks.controllerApi.finish
      .mockRejectedValueOnce({
        code: "longshot_controller_copy_failed",
        message: "untrusted clipboard detail",
      })
      .mockResolvedValueOnce({ action: "save", path: "/tmp/recovered.png" });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();

    expect(document.body.textContent).toContain("Retry copying, save it instead, or discard it");
    expect(document.body.textContent).not.toContain("untrusted clipboard detail");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-copy"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-discard"]')).not.toBeNull();

    await act(async () => document.querySelector('[data-testid="longshot-retry-save"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenCalledTimes(2);
    expect(mocks.controllerApi.finish).toHaveBeenNthCalledWith(2, activation.handle, "save");
    expect(document.body.textContent).toContain("Saved to /tmp/recovered.png. Closing this window");
  });

  it("shows Save-specific Finishing text and keeps both output buttons disabled", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());

    expect(mocks.controllerApi.finish).toHaveBeenCalledWith(activation.handle, "save");
    expect(document.body.textContent).toContain("Finishing the long screenshot and saving it");
    expect(document.querySelector('[data-testid="longshot-copy"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-testid="longshot-save"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-testid="longshot-append"]')?.disabled).toBe(true);

    await act(async () => pending.resolve({ action: "save", path: "/tmp/one.png" }));
    await flush();
  });

  it("moves an explicit Save failure to Any OutputPending and can retry it as Copy", async () => {
    mocks.controllerApi.finish
      .mockRejectedValueOnce({
        code: "longshot_controller_save_failed",
        message: "untrusted filesystem detail",
      })
      .mockResolvedValueOnce({ action: "copy", path: null });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await flush();

    expect(document.body.textContent).toContain("Could not save the long screenshot");
    expect(document.body.textContent).not.toContain("untrusted filesystem detail");
    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).not.toBeNull();

    await act(async () => document.querySelector('[data-testid="longshot-retry-copy"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenNthCalledWith(2, activation.handle, "copy");
    expect(document.body.textContent).toContain("Copied. Closing this window");
  });

  it("keeps Save JoinError recovery Copy-only after a later Copy failure", async () => {
    mocks.controllerApi.finish
      .mockRejectedValueOnce({
        code: "longshot_controller_save_uncertain",
        message: "untrusted save worker detail",
      })
      .mockRejectedValueOnce({
        code: "longshot_controller_copy_failed",
        message: "untrusted copy worker detail",
      });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await flush();

    expect(document.body.textContent).toContain("Saving may have completed");
    expect(document.body.textContent).not.toContain("untrusted save worker detail");
    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();

    await act(async () => document.querySelector('[data-testid="longshot-retry-copy"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenNthCalledWith(2, activation.handle, "copy");
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).toBeNull();
  });

  it.each([
    ["mismatched action", { action: "copy", path: null }],
    ["empty Save path", { action: "save", path: "" }],
  ])("treats a %s finish response as a conservative cleanup error", async (_name, result) => {
    mocks.controllerApi.finish.mockResolvedValue(result);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await flush();

    expect(document.body.textContent).toContain("Restart Clippy");
    expect(document.body.textContent).not.toContain("undefined");
    expect(document.querySelector('[data-testid="longshot-save"]')).toBeNull();
  });

  it("discards an OutputPending artifact through the exact cancel handle", async () => {
    mocks.controllerApi.finish.mockRejectedValue({ code: "longshot_controller_copy_failed" });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();
    await act(async () => document.querySelector('[data-testid="longshot-discard"]').click());
    await flush();

    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(activation.handle);
    expect(document.body.textContent).toContain("Cancelling");
  });

  it.each([
    ["Escape", async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))],
    ["native close", async () => closeRequested()],
  ])("uses the one-shot cancel path when %s discards OutputPending", async (_name, trigger) => {
    mocks.controllerApi.finish.mockRejectedValue({ code: "longshot_controller_copy_failed" });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();
    await act(async () => trigger());
    await flush();

    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(activation.handle);
    expect(document.body.textContent).toContain("Cancelling");
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
  ])("cancels exactly once when %s occurs during Finish and ignores its late result", async (_name, trigger) => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await act(async () => trigger());
    await flush();
    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.cancel).toHaveBeenCalledWith(activation.handle);
    expect(document.body.textContent).toContain("Cancelling");

    await act(async () => pending.resolve({ action: "copy", path: null }));
    await flush();
    expect(document.body.textContent).toContain("Cancelling");
    expect(document.body.textContent).not.toContain("Copied. Closing");
  });

  it("ignores a late finish rejection after cancellation has won", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    await act(async () => pending.reject({ code: "longshot_controller_copy_failed", message: "late detail" }));
    await flush();

    expect(document.body.textContent).toContain("Cancelling");
    expect(document.body.textContent).not.toContain("late detail");
    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).toBeNull();
  });

  it("does not revive a Save result after Cancel has linearized the controller", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    await act(async () => pending.resolve({ action: "save", path: "/tmp/late.png" }));
    await flush();

    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("Cancelling");
    expect(document.body.textContent).not.toContain("/tmp/late.png");
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

  it("invalidates a pending Finish attempt when the controller unmounts", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount(false);

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await act(async () => root.unmount());
    await act(async () => pending.resolve({ action: "copy", path: null }));
    await flush();

    expect(document.body.textContent).not.toContain("Copied. Closing");
  });

  it("invalidates a pending Save attempt when the controller unmounts", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount(false);

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await act(async () => root.unmount());
    await act(async () => pending.resolve({ action: "save", path: "/tmp/late-save.png" }));
    await flush();

    expect(document.body.textContent).not.toContain("/tmp/late-save.png");
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

  it("shows restart guidance without actions after a finish cleanup failure", async () => {
    mocks.controllerApi.finish.mockRejectedValue({
      code: "longshot_controller_cleanup_failed",
      message: "untrusted finish cleanup detail",
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();

    expect(document.body.textContent).toContain("Restart Clippy");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-copy"]')).toBeNull();
    expect(document.body.textContent).not.toContain("untrusted finish cleanup detail");
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
