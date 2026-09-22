import React, { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getConfig: vi.fn(async () => ({ language: "en" })),
  controllerApi: {
    activate: vi.fn(),
    append: vi.fn(),
    autoAppend: vi.fn(),
    undo: vi.fn(),
    preview: vi.fn(),
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
  autoScroll: {
    state: "unsupported",
    reason: "platform_not_implemented",
    directions: [],
  },
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
    mocks.controllerApi.autoAppend.mockResolvedValue(activation.snapshot);
    mocks.controllerApi.undo.mockResolvedValue({
      ...activation.snapshot,
      frameCount: activation.snapshot.frameCount - 1,
      totalHeight: 1200,
    });
    mocks.controllerApi.preview.mockResolvedValue(new Uint8Array([137, 80, 78, 71]).buffer);
    mocks.controllerApi.finish.mockResolvedValue({ action: "copy", path: null, pinLabel: null });
    mocks.controllerApi.ready.mockResolvedValue(undefined);
    mocks.controllerApi.cancel.mockResolvedValue(undefined);
    mocks.controllerApi.onCloseRequested.mockImplementation((callback) => {
      closeRequested = callback;
      return Promise.resolve(vi.fn());
    });
    URL.createObjectURL = vi.fn(() => "blob:longshot-preview");
    URL.revokeObjectURL = vi.fn();
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
    expect(mocks.controllerApi.preview).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.preview).toHaveBeenCalledWith(activation.handle);
    const preview = document.querySelector('[data-testid="longshot-preview"]');
    expect(preview?.getAttribute("role")).toBe("region");
    expect(preview?.getAttribute("aria-label")).toBe("Long screenshot canvas preview");
    expect(preview?.querySelector("img")?.getAttribute("alt")).toBe("Latest long screenshot canvas");
    expect(preview?.querySelector("img")?.getAttribute("src")).toBe("blob:longshot-preview");
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
    expect(URL.createObjectURL.mock.calls[0][0]).toBeInstanceOf(Blob);
    expect(URL.createObjectURL.mock.calls[0][0].type).toBe("image/png");
  });

  it("does not render a fake automatic-scroll action when the backend is unsupported", async () => {
    await mount();

    expect(document.querySelector('[data-testid="longshot-auto-start"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-auto-direction"]')).toBeNull();
    expect(mocks.controllerApi.autoAppend).not.toHaveBeenCalled();
  });

  it("explains a missing macOS permission without exposing an executable action", async () => {
    mocks.controllerApi.activate.mockResolvedValue({
      ...activation,
      autoScroll: {
        state: "permission_required",
        reason: "macos_accessibility_permission",
        directions: [],
      },
    });
    await mount();

    expect(document.querySelector('[data-testid="longshot-auto-permission"]')?.textContent)
      .toContain("Accessibility permission");
    expect(document.querySelector('[data-testid="longshot-auto-start"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-auto-direction"]')).toBeNull();
    expect(mocks.controllerApi.autoAppend).not.toHaveBeenCalled();
  });

  it("runs one controlled automatic step and Stop cancels the queued next step", async () => {
    const automatic = {
      ...activation,
      autoScroll: {
        state: "available",
        reason: null,
        directions: ["down", "up", "right", "left"],
      },
    };
    const pendingStep = deferred();
    mocks.controllerApi.activate.mockResolvedValue(automatic);
    mocks.controllerApi.autoAppend.mockReturnValue(pendingStep.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-auto-start"]').click());
    await flush();
    expect(mocks.controllerApi.autoAppend).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.autoAppend).toHaveBeenCalledWith(automatic.handle, "down");
    expect(document.querySelector('[data-testid="longshot-auto-stop"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-copy"]').disabled).toBe(true);

    await act(async () => pendingStep.resolve({
      ...automatic.snapshot,
      frameCount: 4,
      totalHeight: 2200,
    }));
    await flush();
    await act(async () => document.querySelector('[data-testid="longshot-auto-stop"]').click());
    await new Promise((resolve) => setTimeout(resolve, 700));

    expect(mocks.controllerApi.autoAppend).toHaveBeenCalledTimes(1);
    expect(document.querySelector('[data-testid="longshot-auto-start"]')).not.toBeNull();
    expect(document.body.textContent).toContain("2200");
  });

  it("pauses automatic scrolling on a quality or target failure and keeps manual recovery", async () => {
    mocks.controllerApi.activate.mockResolvedValue({
      ...activation,
      autoScroll: { state: "available", reason: null, directions: ["down"] },
    });
    mocks.controllerApi.autoAppend.mockRejectedValue({
      code: "longshot_auto_target_lost",
      message: "untrusted target detail",
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-auto-start"]').click());
    await flush();

    expect(document.body.textContent).toContain("selected window changed");
    expect(document.body.textContent).not.toContain("untrusted target detail");
    expect(document.querySelector('[data-testid="longshot-auto-start"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-append"]').disabled).toBe(false);
  });

  it("Stop during the preview barrier prevents the first automatic input step", async () => {
    const pendingPreview = deferred();
    mocks.controllerApi.activate.mockResolvedValue({
      ...activation,
      autoScroll: { state: "available", reason: null, directions: ["down"] },
    });
    mocks.controllerApi.preview.mockReturnValue(pendingPreview.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-auto-start"]').click());
    expect(mocks.controllerApi.autoAppend).not.toHaveBeenCalled();
    await act(async () => document.querySelector('[data-testid="longshot-auto-stop"]').click());
    await act(async () => pendingPreview.resolve(new Uint8Array([1]).buffer));
    await flush();

    expect(mocks.controllerApi.autoAppend).not.toHaveBeenCalled();
    expect(document.querySelector('[data-testid="longshot-auto-start"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-append"]').disabled).toBe(false);
  });

  it("keeps the first preview loading until the one-shot ready handshake succeeds", async () => {
    const pendingReady = deferred();
    mocks.controllerApi.ready.mockReturnValue(pendingReady.promise);
    await mount();

    expect(mocks.controllerApi.ready).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.preview).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Loading preview");

    await act(async () => pendingReady.resolve());
    await flush();
    expect(mocks.controllerApi.preview).toHaveBeenCalledTimes(1);
  });

  it("renders a localized unavailable state without exposing a preview rejection", async () => {
    mocks.controllerApi.preview.mockRejectedValue(new Error("raw preview rejection"));
    await mount();

    expect(document.body.textContent).toContain("Preview unavailable");
    expect(document.body.textContent).not.toContain("raw preview rejection");
    expect(document.querySelector('[data-testid="longshot-preview"] img')).toBeNull();
  });

  it("keeps the fixed shrinkable preview and non-shrinking wrapping action CSS contract", () => {
    const css = readFileSync("react/longshot-controller/longshot-controller.css", "utf8");

    expect(css).toContain("height: 100%");
    expect(css).toContain("max-width: 320px");
    expect(css).toContain("max-height: 300px");
    expect(css).toContain("min-height: 0");
    expect(css).toContain("object-fit: contain");
    expect(css).toContain("flex: 0 0 auto");
    expect(css).toContain("flex-wrap: wrap");
  });

  it.each([
    ["Append", "longshot-append", "append"],
    ["Undo", "longshot-undo", "undo"],
    ["Copy", "longshot-copy", "finish"],
    ["Save", "longshot-save", "finish"],
    ["Pin", "longshot-pin", "finish"],
  ])("waits for an in-flight preview before %s IPC", async (_name, testId, method) => {
    const pendingPreview = deferred();
    const replacementPreview = deferred();
    mocks.controllerApi.preview
      .mockReturnValueOnce(pendingPreview.promise)
      .mockReturnValueOnce(replacementPreview.promise);
    await mount();

    await act(async () => document.querySelector(`[data-testid="${testId}"]`).click());
    expect(mocks.controllerApi[method]).not.toHaveBeenCalled();

    await act(async () => pendingPreview.resolve(new Uint8Array([1]).buffer));
    await flush();
    expect(mocks.controllerApi[method]).toHaveBeenCalledTimes(1);
    expect(URL.createObjectURL).not.toHaveBeenCalled();
  });

  it("lets Cancel preempt an in-flight preview and discards its late bytes before URL creation", async () => {
    const pendingPreview = deferred();
    mocks.controllerApi.preview.mockReturnValue(pendingPreview.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-cancel"]').click());
    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);

    await act(async () => pendingPreview.resolve(new Uint8Array([1]).buffer));
    await flush();
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Cancelling");
  });

  it("reissues the same snapshot preview after a recoverable Append failure", async () => {
    const stalePreview = deferred();
    const replacementPreview = deferred();
    mocks.controllerApi.preview
      .mockReturnValueOnce(stalePreview.promise)
      .mockReturnValueOnce(replacementPreview.promise);
    mocks.controllerApi.append.mockRejectedValue({
      code: "longshot_estimate_low_texture",
      message: "untrusted append detail",
    });
    URL.createObjectURL.mockReturnValueOnce("blob:append-recovered-preview");
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    expect(mocks.controllerApi.append).not.toHaveBeenCalled();
    await act(async () => stalePreview.resolve(new Uint8Array([1]).buffer));
    await flush();

    expect(mocks.controllerApi.append).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.preview).toHaveBeenCalledTimes(2);
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Could not append another frame");

    await act(async () => replacementPreview.resolve(new Uint8Array([2]).buffer));
    await flush();
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:append-recovered-preview");
  });

  it("reissues the same snapshot preview after a recoverable Finish failure", async () => {
    const stalePreview = deferred();
    const replacementPreview = deferred();
    mocks.controllerApi.preview
      .mockReturnValueOnce(stalePreview.promise)
      .mockReturnValueOnce(replacementPreview.promise);
    mocks.controllerApi.finish.mockRejectedValue({
      code: "longshot_estimate_low_texture",
      message: "untrusted finish detail",
    });
    URL.createObjectURL.mockReturnValueOnce("blob:finish-recovered-preview");
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    expect(mocks.controllerApi.finish).not.toHaveBeenCalled();
    await act(async () => stalePreview.resolve(new Uint8Array([1]).buffer));
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.preview).toHaveBeenCalledTimes(2);
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Could not finish the long screenshot");

    await act(async () => replacementPreview.resolve(new Uint8Array([2]).buffer));
    await flush();
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:finish-recovered-preview");
  });

  it("keeps a handled preview unchanged after a recoverable Append failure", async () => {
    mocks.controllerApi.append.mockRejectedValue({
      code: "longshot_estimate_low_texture",
      message: "untrusted append detail",
    });
    URL.createObjectURL.mockReturnValueOnce("blob:handled-before-append");
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await flush();

    expect(mocks.controllerApi.preview).toHaveBeenCalledTimes(1);
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:handled-before-append");
    expect(document.body.textContent).toContain("Could not append another frame");
  });

  it("keeps a handled preview unchanged after a recoverable Finish failure", async () => {
    mocks.controllerApi.finish.mockRejectedValue({
      code: "longshot_estimate_low_texture",
      message: "untrusted finish detail",
    });
    URL.createObjectURL.mockReturnValueOnce("blob:handled-before-finish");
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();

    expect(mocks.controllerApi.preview).toHaveBeenCalledTimes(1);
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:handled-before-finish");
    expect(document.body.textContent).toContain("Could not finish the long screenshot");
  });

  it("replaces the owned URL after Append and revokes the old URL exactly once", async () => {
    const nextSnapshot = { ...activation.snapshot, frameCount: 4, totalHeight: 2200 };
    const refreshedPreview = deferred();
    mocks.controllerApi.append.mockResolvedValue(nextSnapshot);
    mocks.controllerApi.preview
      .mockResolvedValueOnce(new Uint8Array([1]).buffer)
      .mockReturnValueOnce(refreshedPreview.promise);
    URL.createObjectURL
      .mockReturnValueOnce("blob:preview-first")
      .mockReturnValueOnce("blob:preview-second");
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:preview-first");
    await act(async () => refreshedPreview.resolve(new Uint8Array([2]).buffer));
    await flush();

    expect(mocks.controllerApi.preview).toHaveBeenCalledTimes(2);
    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:preview-second");
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:preview-first");
  });

  it("keeps the previous image when a refreshed preview fails", async () => {
    const nextSnapshot = { ...activation.snapshot, frameCount: 4, totalHeight: 2200 };
    mocks.controllerApi.append.mockResolvedValue(nextSnapshot);
    mocks.controllerApi.preview
      .mockResolvedValueOnce(new Uint8Array([1]).buffer)
      .mockRejectedValueOnce(new Error("raw preview failure"));
    URL.createObjectURL.mockReturnValueOnce("blob:preview-kept");
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await flush();

    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:preview-kept");
    expect(document.body.textContent).not.toContain("raw preview failure");
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
    expect(document.querySelector('[data-testid="longshot-copy"]')).not.toBeNull();
  });

  it("recovers from object URL creation failure after the next successful Append", async () => {
    const nextSnapshot = { ...activation.snapshot, frameCount: 4, totalHeight: 2200 };
    mocks.controllerApi.append.mockResolvedValue(nextSnapshot);
    URL.createObjectURL
      .mockImplementationOnce(() => { throw new Error("raw object URL failure"); })
      .mockReturnValueOnce("blob:preview-recovered");
    await mount();

    expect(document.body.textContent).toContain("Preview unavailable");
    expect(document.body.textContent).not.toContain("raw object URL failure");
    await act(async () => document.querySelector('[data-testid="longshot-append"]').click());
    await flush();

    expect(document.querySelector(".longshot-preview img")?.getAttribute("src"))
      .toBe("blob:preview-recovered");
  });

  it.each([
    ["Finish", "longshot-copy", "finish"],
    ["Cancel", "longshot-cancel", "cancel"],
  ])("releases the owned preview exactly once after successful %s", async (_name, testId) => {
    URL.createObjectURL.mockReturnValueOnce("blob:preview-terminal");
    await mount();

    await act(async () => document.querySelector(`[data-testid="${testId}"]`).click());
    await flush();

    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:preview-terminal");
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
    mocks.controllerApi.finish.mockResolvedValue({
      action: "save", path: "/tmp/长截图/结果.png", pinLabel: null,
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.finish).toHaveBeenCalledWith(activation.handle, "save");
    expect(document.body.textContent).toContain("Saved to /tmp/长截图/结果.png. Closing this window");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-save"]')).toBeNull();
  });

  it("finishes with Pin through the shared barrier without exposing its internal label", async () => {
    mocks.controllerApi.finish.mockResolvedValue({
      action: "pin", path: null, pinLabel: "pin-image-internal-42",
    });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-pin"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenCalledWith(activation.handle, "pin");
    expect(document.body.textContent).toContain("Pinned. Closing this window");
    expect(document.body.textContent).not.toContain("pin-image-internal-42");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-pin"]')).toBeNull();
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

  it("prevents same-tick Copy/Save/Pin reentry and disables every output action while Finishing", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount();

    await act(async () => {
      document.querySelector('[data-testid="longshot-copy"]').click();
      document.querySelector('[data-testid="longshot-save"]').click();
      document.querySelector('[data-testid="longshot-pin"]').click();
    });

    expect(mocks.controllerApi.finish).toHaveBeenCalledTimes(1);
    expect(mocks.controllerApi.finish).toHaveBeenCalledWith(activation.handle, "copy");
    expect(document.querySelector('[data-testid="longshot-copy"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-testid="longshot-save"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-testid="longshot-pin"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-testid="longshot-append"]')?.disabled).toBe(true);
    expect(document.body.textContent).toContain("Finishing the long screenshot and copying it");

    await act(async () => pending.resolve({ action: "copy", path: null, pinLabel: null }));
    await flush();
  });

  it("moves a Copy failure to Any OutputPending and can retry it as Save without reopening Append", async () => {
    mocks.controllerApi.finish
      .mockRejectedValueOnce({
        code: "longshot_controller_copy_failed",
        message: "untrusted clipboard detail",
      })
      .mockResolvedValueOnce({ action: "save", path: "/tmp/recovered.png", pinLabel: null });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await flush();

    expect(document.body.textContent).toContain("An output attempt did not finish");
    expect(document.body.textContent).not.toContain("untrusted clipboard detail");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-copy"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-pin"]')).not.toBeNull();
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

    await act(async () => pending.resolve({ action: "save", path: "/tmp/one.png", pinLabel: null }));
    await flush();
  });

  it("moves an explicit Save failure to Any OutputPending and can retry it as Copy", async () => {
    mocks.controllerApi.finish
      .mockRejectedValueOnce({
        code: "longshot_controller_save_failed",
        message: "untrusted filesystem detail",
      })
      .mockResolvedValueOnce({ action: "copy", path: null, pinLabel: null });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await flush();

    expect(document.body.textContent).toContain("Could not save the long screenshot");
    expect(document.body.textContent).not.toContain("untrusted filesystem detail");
    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-pin"]')).not.toBeNull();

    await act(async () => document.querySelector('[data-testid="longshot-retry-copy"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenNthCalledWith(2, activation.handle, "copy");
    expect(document.body.textContent).toContain("Copied. Closing this window");
  });

  it("moves an explicit Pin failure to Any OutputPending and can retry it", async () => {
    mocks.controllerApi.finish
      .mockRejectedValueOnce({
        code: "longshot_controller_pin_failed",
        message: "untrusted window detail",
      })
      .mockResolvedValueOnce({ action: "pin", path: null, pinLabel: "pin-image-10" });
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-pin"]').click());
    await flush();

    expect(document.body.textContent).toContain("Could not pin the long screenshot");
    expect(document.body.textContent).not.toContain("untrusted window detail");
    expect(document.body.textContent).toContain("An output attempt did not finish");
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-pin"]')).not.toBeNull();

    await act(async () => document.querySelector('[data-testid="longshot-retry-pin"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenNthCalledWith(2, activation.handle, "pin");
    expect(document.body.textContent).toContain("Pinned. Closing this window");
    expect(document.body.textContent).not.toContain("pin-image-10");
  });

  it.each([
    [
      "Pin then Save",
      "pin",
      "longshot_controller_pin_uncertain",
      "save",
      "longshot_controller_save_uncertain",
    ],
    [
      "Save then Pin",
      "save",
      "longshot_controller_save_uncertain",
      "pin",
      "longshot_controller_pin_uncertain",
    ],
  ])("monotonically reduces %s uncertain outputs to Copy-only", async (
    _name,
    firstAction,
    firstError,
    secondAction,
    secondError,
  ) => {
    mocks.controllerApi.finish
      .mockRejectedValueOnce({ code: firstError, message: "untrusted first detail" })
      .mockRejectedValueOnce({ code: secondError, message: "untrusted second detail" })
      .mockRejectedValueOnce({ code: "longshot_controller_copy_failed", message: "untrusted copy detail" });
    await mount();

    await act(async () => document.querySelector(`[data-testid="longshot-${firstAction}"]`).click());
    await flush();

    const removedFirst = firstAction === "pin" ? "save" : "pin";
    expect(document.querySelector(`[data-testid="longshot-retry-${firstAction}"]`)).toBeNull();
    expect(document.querySelector(`[data-testid="longshot-retry-${removedFirst}"]`)).not.toBeNull();
    await act(async () => document.querySelector(`[data-testid="longshot-retry-${secondAction}"]`).click());
    await flush();

    expect(document.querySelector('[data-testid="longshot-retry-copy"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-pin"]')).toBeNull();
    await act(async () => document.querySelector('[data-testid="longshot-retry-copy"]').click());
    await flush();

    // 已被 uncertain 移除的动作不会因之后的业务失败重新出现。
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-pin"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();
  });

  it("keeps Save JoinError recovery Copy+Pin after a later Copy failure", async () => {
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
    expect(document.querySelector('[data-testid="longshot-retry-pin"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="longshot-append"]')).toBeNull();

    await act(async () => document.querySelector('[data-testid="longshot-retry-copy"]').click());
    await flush();

    expect(mocks.controllerApi.finish).toHaveBeenNthCalledWith(2, activation.handle, "copy");
    expect(document.querySelector('[data-testid="longshot-retry-save"]')).toBeNull();
    expect(document.querySelector('[data-testid="longshot-retry-pin"]')).not.toBeNull();
  });

  it.each([
    ["mismatched action", "save", { action: "copy", path: null, pinLabel: null }],
    ["empty Save path", "save", { action: "save", path: "", pinLabel: null }],
    ["Copy with a Pin label", "copy", { action: "copy", path: null, pinLabel: "pin-image-1" }],
    ["Save with a Pin label", "save", { action: "save", path: "/tmp/cross.png", pinLabel: "pin-image-1" }],
    ["Pin with a path", "pin", { action: "pin", path: "/tmp/cross.png", pinLabel: "pin-image-1" }],
    ["empty Pin label", "pin", { action: "pin", path: null, pinLabel: "" }],
    ["missing Pin label", "pin", { action: "pin", path: null }],
  ])("treats a %s finish response as a conservative cleanup error", async (_name, action, result) => {
    mocks.controllerApi.finish.mockResolvedValue(result);
    await mount();

    await act(async () => document.querySelector(`[data-testid="longshot-${action}"]`).click());
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

    await act(async () => pending.resolve({ action: "copy", path: null, pinLabel: null }));
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
    await act(async () => pending.resolve({ action: "save", path: "/tmp/late.png", pinLabel: null }));
    await flush();

    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("Cancelling");
    expect(document.body.textContent).not.toContain("/tmp/late.png");
  });

  it.each([
    ["Cancel", async () => document.querySelector('[data-testid="longshot-cancel"]').click()],
    ["Escape", async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))],
    ["native close", async () => closeRequested()],
  ])("does not revive a Pin result after %s has linearized the controller", async (_name, trigger) => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount();

    await act(async () => document.querySelector('[data-testid="longshot-pin"]').click());
    await act(async () => trigger());
    await act(async () => pending.resolve({ action: "pin", path: null, pinLabel: "pin-image-late" }));
    await flush();

    expect(mocks.controllerApi.cancel).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("Cancelling");
    expect(document.body.textContent).not.toContain("Pinned. Closing");
    expect(document.body.textContent).not.toContain("pin-image-late");
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
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:longshot-preview");
  });

  it("invalidates a pending Finish attempt when the controller unmounts", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount(false);

    await act(async () => document.querySelector('[data-testid="longshot-copy"]').click());
    await act(async () => root.unmount());
    await act(async () => pending.resolve({ action: "copy", path: null, pinLabel: null }));
    await flush();

    expect(document.body.textContent).not.toContain("Copied. Closing");
  });

  it("invalidates a pending Save attempt when the controller unmounts", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount(false);

    await act(async () => document.querySelector('[data-testid="longshot-save"]').click());
    await act(async () => root.unmount());
    await act(async () => pending.resolve({
      action: "save", path: "/tmp/late-save.png", pinLabel: null,
    }));
    await flush();

    expect(document.body.textContent).not.toContain("/tmp/late-save.png");
  });

  it("invalidates a pending Pin attempt when the controller unmounts", async () => {
    const pending = deferred();
    mocks.controllerApi.finish.mockReturnValue(pending.promise);
    await mount(false);

    await act(async () => document.querySelector('[data-testid="longshot-pin"]').click());
    await act(async () => root.unmount());
    await act(async () => pending.resolve({ action: "pin", path: null, pinLabel: "pin-image-late" }));
    await flush();

    expect(document.body.textContent).not.toContain("Pinned. Closing");
    expect(document.body.textContent).not.toContain("pin-image-late");
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
