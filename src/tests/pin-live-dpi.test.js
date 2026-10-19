import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  getCurrentWindowLabel: () => "pin-dpi-test",
  startDraggingCurrentWindow: async () => {},
}));
vi.mock("../react/pin/api.ts", () => ({ pinApi: {} }));

import { App } from "../react/pin/App.tsx";
import { init } from "../i18n/i18n.js";

const payload = {
  label: "pin-dpi-test", kind: "image", text: null, color: null,
  contentWidth: 800, contentHeight: 600, scale: 1, opacity: 1,
  locked: false, above: false, workspaceId: null, workspaceGroupId: null,
  canSave: true, position: null, deviceScale: 1.5, bufferScale: 1.5,
  initialProject: null,
};

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(os = "windows") {
  let notify = () => {};
  const dispose = vi.fn();
  const api = {
    get: vi.fn(async () => payload),
    imageUrl: vi.fn(() => "http://pin-frame.localhost/pin-dpi-test?revision=0"),
    platform: vi.fn(async () => ({
      operating_system: os,
      capabilities: { always_on_top: { state: "available", reason: null } },
    })),
    displayScale: vi.fn(async () => 1.5),
    onDisplayScaleChanged: vi.fn(async (callback) => { notify = callback; return dispose; }),
    ready: vi.fn(async () => {}),
    toolbarBounds: vi.fn(async () => ({ x: 0, y: 0, width: 868, height: 672 })),
    onSharpened: vi.fn(async () => () => {}),
    onAlreadyOpen: vi.fn(async () => () => {}),
    onWorkspaceChanged: vi.fn(async () => () => {}),
    onCloseRequested: vi.fn(async () => () => {}),
    groups: vi.fn(async () => []),
    close: vi.fn(async () => {}),
    sourceImage: vi.fn(async () => null),
  };
  return {
    api, dispose, notify: (value) => notify(value),
    services: { api, windowLabel: () => "pin-dpi-test", startDragging: async () => {},
      viewport: () => ({ width: 2048, height: 1536 }) },
  };
}

async function flush() {
  await act(async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); });
}

describe("WIN-PIN-LIVE-DPI-01 real Pin App filter contract", () => {
  let root;
  let mounted;
  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    init("en");
    root = createRoot(document.getElementById("root"));
    mounted = true;
  });
  afterEach(async () => {
    if (mounted) await act(async () => root.unmount());
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  async function mountImage(test) {
    await act(async () => root.render(React.createElement(App, { services: test.services })));
    await flush();
    const image = document.querySelector(".pin-media img");
    expect(image).not.toBeNull();
    Object.defineProperties(image, {
      naturalWidth: { value: 1200 }, naturalHeight: { value: 900 },
    });
    await act(async () => image.dispatchEvent(new Event("load")));
    await flush();
    return image;
  }

  it("uses current 100% DPI instead of the 150% creation payload", async () => {
    const test = fixture();
    test.api.displayScale.mockResolvedValue(1);
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("auto");
    expect(document.querySelector(".pin-media").style.maxWidth).toBe("800px");
    expect(image.src).toContain("revision=0");
  });

  it("tracks 150/100/125/150% events without a DOM resize or source reload", async () => {
    const test = fixture();
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("pixelated");
    for (const [scale, filter] of [[1, "auto"], [1.25, "auto"], [1.5, "pixelated"]]) {
      await act(async () => test.notify(scale));
      expect(image.style.imageRendering).toBe(filter);
    }
    expect(test.api.get).toHaveBeenCalledTimes(1);
    expect(test.api.imageUrl).toHaveBeenCalledTimes(1);
  });

  it("does not overwrite a newer scale event with a late first read", async () => {
    const test = fixture();
    const read = deferred();
    test.api.displayScale.mockReturnValue(read.promise);
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("auto");
    await act(async () => test.notify(1));
    await act(async () => read.resolve(1.5));
    await flush();
    expect(image.style.imageRendering).toBe("auto");
  });

  it("retains an event that arrives before the initial Pin payload", async () => {
    const test = fixture();
    const load = deferred();
    test.api.get.mockReturnValue(load.promise);
    await act(async () => root.render(React.createElement(App, { services: test.services })));
    await flush();
    await act(async () => test.notify(1));
    await act(async () => load.resolve(payload));
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("auto");
    expect(test.api.get).toHaveBeenCalledTimes(1);
  });

  it("uses smoothing while the first native read is pending", async () => {
    const test = fixture();
    const read = deferred();
    test.api.displayScale.mockReturnValue(read.promise);
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("auto");
    await act(async () => read.resolve(1.5));
    await flush();
    expect(image.style.imageRendering).toBe("pixelated");
  });

  it("rejects invalid native scale values and recovers on a valid event", async () => {
    const test = fixture();
    const image = await mountImage(test);
    for (const scale of [0, -1, NaN, Infinity, "1.5", null, undefined]) {
      await act(async () => test.notify(scale));
      expect(image.style.imageRendering).toBe("auto");
    }
    await act(async () => test.notify(1.5));
    expect(image.style.imageRendering).toBe("pixelated");
  });

  it("does not reuse creation DPI after a failed read; events can recover", async () => {
    const test = fixture();
    test.api.displayScale.mockRejectedValue(new Error("native read denied"));
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("auto");
    await act(async () => test.notify(1.5));
    expect(image.style.imageRendering).toBe("pixelated");
  });

  it("uses smoothing if the native subscription fails", async () => {
    const test = fixture();
    test.api.onDisplayScaleChanged.mockRejectedValue(new Error("listen denied"));
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("auto");
    expect(test.api.displayScale).not.toHaveBeenCalled();
  });

  it("cleans a subscription that completes after unmount without querying", async () => {
    const test = fixture();
    const registration = deferred();
    test.api.onDisplayScaleChanged.mockReturnValue(registration.promise);
    await mountImage(test);
    await act(async () => root.unmount());
    mounted = false;
    await act(async () => registration.resolve(test.dispose));
    await flush();
    expect(test.dispose).toHaveBeenCalledTimes(1);
    expect(test.api.displayScale).not.toHaveBeenCalled();
  });

  it("cleans an active subscription even with an unresolved native read", async () => {
    const test = fixture();
    test.api.displayScale.mockReturnValue(deferred().promise);
    await mountImage(test);
    await act(async () => root.unmount());
    mounted = false;
    expect(test.dispose).toHaveBeenCalledTimes(1);
  });

  it.each(["linux", "macos"])("keeps %s payload rendering without native scale access", async (os) => {
    const test = fixture(os);
    const image = await mountImage(test);
    expect(image.style.imageRendering).toBe("pixelated");
    expect(test.api.onDisplayScaleChanged).not.toHaveBeenCalled();
    expect(test.api.displayScale).not.toHaveBeenCalled();
  });
});
