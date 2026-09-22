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
    scan: vi.fn(),
    recordingCapabilities: vi.fn(),
    startRecording: vi.fn(),
    copyText: vi.fn(),
    openLongshot: vi.fn(),
    onHandoff: vi.fn(),
    retry: vi.fn(),
    closeUninitialized: vi.fn(),
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
  intent: "screenshot",
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
    mocks.getCurrentWindowLabel.mockReturnValue("capture-overlay-session-1-0");
    mocks.overlayApi.ready.mockResolvedValue(undefined);
    mocks.overlayApi.cancel.mockResolvedValue(undefined);
    mocks.overlayApi.onHandoff.mockResolvedValue(() => {});
    mocks.overlayApi.retry.mockResolvedValue({ action: "copy", path: null, pinLabel: null });
    mocks.overlayApi.closeUninitialized.mockResolvedValue(undefined);
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
    mocks.overlayApi.scan.mockResolvedValue({ results: [], limited: false });
    mocks.overlayApi.recordingCapabilities.mockResolvedValue({
      audioModes: ["none"],
      deviceCatalog: {
        catalogId: "audio-catalog-0000000000000001",
        systemAudioDevices: [],
        microphoneDevices: [],
      },
      deviceEnumerationFailed: false,
    });
    mocks.overlayApi.startRecording.mockResolvedValue(undefined);
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

  it("keeps the recording entry isolated from screenshot tools and starts the selected area", async () => {
    mocks.getCurrentWindowLabel.mockReturnValue("recording-overlay-session-1-0");
    await mount({ intent: "recording" });
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    expect(button("Start recording")).not.toBeNull();
    expect(button("Cancel")).not.toBeNull();
    expect(button("Copy")).toBeNull();
    expect(button("Scan QR/barcode")).toBeNull();
    expect(button("Long screenshot")).toBeNull();
    expect(button("Blur")).toBeNull();

    await act(async () => button("Start recording").click());

    expect(mocks.overlayApi.startRecording).toHaveBeenCalledWith(
      {
        sessionId: "session-1",
        monitorId: 0,
        x: 10,
        y: 10,
        width: 90,
        height: 70,
      },
      {
        mode: "none",
        catalogId: "audio-catalog-0000000000000001",
        systemDeviceId: null,
        microphoneDeviceId: null,
      },
    );
    expect(button("Start recording").disabled).toBe(true);
  });

  it("uses only backend-advertised recording audio modes", async () => {
    mocks.getCurrentWindowLabel.mockReturnValue("recording-overlay-session-1-0");
    mocks.overlayApi.recordingCapabilities.mockResolvedValue({
      audioModes: ["none", "systemAudio", "microphone", "systemAndMicrophone"],
      deviceCatalog: {
        catalogId: "audio-catalog-0000000000000002",
        systemAudioDevices: [],
        microphoneDevices: [],
      },
      deviceEnumerationFailed: false,
    });
    await mount({ intent: "recording" });
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    expect(button("Recording audio: None")).not.toBeNull();
    await act(async () => button("Recording audio: None").click());
    expect(button("Recording audio: System audio")).not.toBeNull();
    await act(async () => button("Recording audio: System audio").click());
    expect(button("Recording audio: Microphone")).not.toBeNull();
    await act(async () => button("Recording audio: Microphone").click());
    expect(button("Recording audio: System audio + microphone")).not.toBeNull();

    await act(async () => button("Start recording").click());
    expect(mocks.overlayApi.startRecording).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "session-1", width: 90, height: 70 }),
      {
        mode: "systemAndMicrophone",
        catalogId: "audio-catalog-0000000000000002",
        systemDeviceId: null,
        microphoneDeviceId: null,
      },
    );
  });

  it("falls back to silent recording when capability loading fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.getCurrentWindowLabel.mockReturnValue("recording-overlay-session-1-0");
    mocks.overlayApi.recordingCapabilities.mockRejectedValue(new Error("unavailable"));
    await mount({ intent: "recording" });
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    expect(document.querySelector(".recording-audio-mode")).toBeNull();
    await act(async () => button("Start recording").click());
    expect(mocks.overlayApi.startRecording).toHaveBeenCalledWith(
      expect.any(Object),
      { mode: "none", catalogId: null, systemDeviceId: null, microphoneDeviceId: null },
    );
    warn.mockRestore();
  });

  it("selects independent system and microphone devices from opaque catalogs", async () => {
    mocks.getCurrentWindowLabel.mockReturnValue("recording-overlay-session-1-0");
    mocks.overlayApi.recordingCapabilities.mockResolvedValue({
      audioModes: ["none", "systemAndMicrophone"],
      deviceCatalog: {
        catalogId: "audio-catalog-0000000000000003",
        systemAudioDevices: [{
          id: "audio-device-0000000000000003-00",
          label: "Speakers",
          isDefault: true,
        }],
        microphoneDevices: [{
          id: "audio-device-0000000000000003-01",
          label: "USB microphone",
          isDefault: false,
        }],
      },
      deviceEnumerationFailed: false,
    });
    await mount({ intent: "recording" });
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    await act(async () => button("Recording audio: None").click());
    await act(async () => button("Choose audio devices").click());
    const system = document.querySelector('select[aria-label="System audio device"]');
    const microphone = document.querySelector('select[aria-label="Microphone device"]');
    await act(async () => {
      system.value = "audio-device-0000000000000003-00";
      system.dispatchEvent(new Event("change", { bubbles: true }));
      microphone.value = "audio-device-0000000000000003-01";
      microphone.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => button("Start recording").click());

    expect(mocks.overlayApi.startRecording).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "session-1" }),
      {
        mode: "systemAndMicrophone",
        catalogId: "audio-catalog-0000000000000003",
        systemDeviceId: "audio-device-0000000000000003-00",
        microphoneDeviceId: "audio-device-0000000000000003-01",
      },
    );
  });

  it("restores the recording selection when native startup fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.getCurrentWindowLabel.mockReturnValue("recording-overlay-session-1-0");
    mocks.overlayApi.recordingCapabilities
      .mockResolvedValueOnce({
        audioModes: ["none"],
        deviceCatalog: {
          catalogId: "audio-catalog-0000000000000004",
          systemAudioDevices: [],
          microphoneDevices: [],
        },
        deviceEnumerationFailed: false,
      })
      .mockResolvedValueOnce({
        audioModes: ["none"],
        deviceCatalog: {
          catalogId: "audio-catalog-0000000000000005",
          systemAudioDevices: [],
          microphoneDevices: [],
        },
        deviceEnumerationFailed: false,
      });
    mocks.overlayApi.startRecording.mockRejectedValue(new Error("backend unavailable"));
    await mount({ intent: "recording" });
    await drag({ x: 10, y: 10 }, { x: 100, y: 80 });

    await act(async () => button("Start recording").click());
    await flush();

    expect(document.querySelector(".overlay-error")?.textContent).toContain(
      "Could not start recording",
    );
    expect(button("Start recording").disabled).toBe(false);
    expect(selectionRect()).toEqual({ x: 10, y: 10, width: 90, height: 70 });
    expect(mocks.overlayApi.recordingCapabilities).toHaveBeenCalledTimes(2);
    warn.mockRestore();
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

  it("keeps size-label and toolbar drags outside selection ownership while tools and sliders work", async () => {
    await mount(); await drag({ x: 20, y: 40 }, { x: 160, y: 120 });
    const before = selectionRect();
    for (const target of [document.querySelector(".selection-size"), document.querySelector(".overlay-toolbar")]) {
      await act(async () => {
        for (const [type, x, y] of [["pointerdown", 20, 20], ["pointermove", 100, 50], ["pointerup", 100, 50]]) {
          target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y }));
        }
      });
      expect(selectionRect()).toEqual(before);
    }
    await act(async () => button("Pen").click());
    expect(document.querySelector(".overlay-root").dataset.tool).toBe("pen");
    await act(async () => changeRange(document.querySelector('input[type="range"]'), 12));
    expect(document.querySelector('input[type="range"]').value).toBe("12");
    expect(selectionRect()).toEqual(before);
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();
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

  it("scans the authoritative selection and copies only an explicit result", async () => {
    mocks.overlayApi.scan.mockResolvedValue({
      limited: false,
      results: [{ format: "qr_code", text: "https://example.test/<unsafe>", points: [] }],
    });
    mocks.overlayApi.copyText.mockResolvedValue(undefined);
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => button("Scan QR or barcode").click());
    await flush();

    expect(mocks.overlayApi.scan).toHaveBeenCalledWith({
      sessionId: "session-1",
      monitorId: 0,
      x: 10,
      y: 10,
      width: 100,
      height: 80,
    });
    expect(document.querySelector(".scan-result pre").textContent)
      .toBe("https://example.test/<unsafe>");
    expect(document.querySelector(".scan-result a")).toBeNull();
    expect(mocks.overlayApi.copyText).not.toHaveBeenCalled();

    await act(async () => button("Copy code 1").click());
    await flush();
    expect(mocks.overlayApi.copyText).toHaveBeenCalledWith("https://example.test/<unsafe>");
  });

  it("drops a late scan result after the selection moves", async () => {
    let resolveScan;
    mocks.overlayApi.scan.mockReturnValue(new Promise((resolve) => (resolveScan = resolve)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Scan QR or barcode").click());

    await drag({ x: 40, y: 40 }, { x: 60, y: 60 });
    await act(async () => resolveScan({
      limited: false,
      results: [{ format: "qr_code", text: "stale", points: [] }],
    }));
    await flush();

    expect(document.querySelector(".scan-popover")).toBeNull();
    expect(document.body.textContent).not.toContain("stale");
  });

  it("blocks same-tick output during scan and restores ordinary output after failure", async () => {
    let rejectScan;
    mocks.overlayApi.scan.mockReturnValue(new Promise((_, reject) => (rejectScan = reject)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });

    await act(async () => {
      button("Scan QR or barcode").click();
      button("Copy").click();
    });
    expect(mocks.overlayApi.commit).not.toHaveBeenCalled();

    await act(async () => rejectScan({ code: "decode_failed", detail: "private" }));
    await flush();
    expect(document.querySelector(".scan-failure").textContent)
      .toBe("Could not scan this selection.");
    expect(button("Copy").disabled).toBe(false);

    await act(async () => button("Copy").click());
    await flush();
    expect(mocks.overlayApi.commit).toHaveBeenCalledTimes(1);
  });

  it("drops a late scan result after capture cancellation", async () => {
    let resolveScan;
    mocks.overlayApi.scan.mockReturnValue(new Promise((resolve) => (resolveScan = resolve)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Scan QR or barcode").click());
    await act(async () => button("Cancel").click());
    await act(async () => resolveScan({
      limited: false,
      results: [{ format: "qr_code", text: "late-after-close", points: [] }],
    }));
    await flush();

    expect(mocks.overlayApi.cancel).toHaveBeenCalledWith("session-1");
    expect(document.body.textContent).not.toContain("late-after-close");
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
    expect(document.querySelector(".overlay-canvas").classList.contains("is-idle")).toBe(false);
    expect(document.querySelector(".overlay-frame-host").classList.contains("is-composited")).toBe(true);
    expect(button("Long screenshot").getAttribute("aria-description")).toBe(
      "Long screenshot starts from the original selection and cannot include current annotations or image adjustments.",
    );
    expect(button("Copy").disabled).toBe(false);

    await act(async () => changeRange(corners, 0));
    expect(button("Long screenshot").disabled).toBe(false);
    expect(document.querySelector(".overlay-canvas").classList.contains("is-idle")).toBe(true);
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
  it("exits an initialization error by its current native window without requiring payload", async () => {
    mocks.overlayApi.get.mockRejectedValue(new Error("payload unavailable"));
    await act(async () => root.render(React.createElement(App)));
    await flush();
    expect(document.querySelector(".overlay-error").textContent).toContain("payload unavailable");
    expect(document.querySelector(".overlay-error button").textContent).toBe("Cancel");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(mocks.overlayApi.closeUninitialized).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.cancel).not.toHaveBeenCalled();
  });

  it("returns to editing after rendering fails and preserves annotations for a fresh commit", async () => {
    mocks.overlayApi.commit.mockRejectedValueOnce({ message: "render failed", outputPending: false, retryActions: [] });
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Pen").click());
    await drag({ x: 24, y: 24 }, { x: 56, y: 56 });
    await act(async () => button("Copy").click());
    expect(document.querySelector(".overlay-output-failure")).toBeNull();
    expect(button("Copy").disabled).toBe(false);
    await act(async () => button("Copy").click());
    expect(mocks.overlayApi.commit).toHaveBeenCalledTimes(2);
    expect(mocks.overlayApi.commit.mock.calls[1][2].annotations).toHaveLength(1);
  });

  it("retains failed output, locks edits, and retries the artifact without another commit", async () => {
    mocks.overlayApi.commit.mockRejectedValueOnce({ message: "disk full", outputPending: true, retryActions: ["copy", "save", "pin"] });
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Save").click());
    const before = selectionRect();
    expect(document.querySelector(".overlay-output-failure").textContent).toContain("disk full");
    expect(button("Copy").disabled).toBe(true);
    await drag({ x: 125, y: 105 }, { x: 185, y: 140 });
    expect(selectionRect()).toEqual(before);
    let resolve;
    mocks.overlayApi.retry.mockReturnValue(new Promise((done) => (resolve = done)));
    const retry = [...document.querySelectorAll(".overlay-output-actions button")].find((node) => node.textContent === "Save");
    await act(async () => { retry.click(); retry.click(); });
    expect(mocks.overlayApi.retry).toHaveBeenCalledTimes(1);
    expect(mocks.overlayApi.retry).toHaveBeenCalledWith("save");
    expect(mocks.overlayApi.commit).toHaveBeenCalledTimes(1);
    await act(async () => resolve({ action: "save", path: "/tmp/complete.png", pinLabel: null }));
  });

  it("offers only safe Copy after uncertain output and Escape explicitly discards it", async () => {
    mocks.overlayApi.commit.mockRejectedValueOnce({ message: "pin result unknown", outputPending: true, retryActions: ["copy"] });
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Pin").click());
    expect([...document.querySelectorAll(".overlay-output-actions button")].map((node) => node.textContent)).toEqual(["Copy", "Discard image"]);
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(mocks.overlayApi.cancel).toHaveBeenCalledWith("session-1");
    expect(mocks.overlayApi.retry).not.toHaveBeenCalled();
  });

  it("keeps an unconfirmed IPC failure locked and never offers another Save or Pin", async () => {
    mocks.overlayApi.commit.mockRejectedValueOnce(new Error("reply channel lost"));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    await act(async () => button("Save").click());
    const actions = () => [...document.querySelectorAll(".overlay-output-actions button")];
    expect(actions().map((node) => node.textContent)).toEqual(["Copy", "Discard image"]);
    expect(button("Save").disabled).toBe(true);
    mocks.overlayApi.retry.mockRejectedValueOnce(new Error("retry reply lost"));
    await act(async () => actions()[0].click());
    expect(actions().map((node) => node.textContent)).toEqual(["Copy", "Discard image"]);
    expect(mocks.overlayApi.retry).toHaveBeenCalledWith("copy");
    expect(mocks.overlayApi.commit).toHaveBeenCalledTimes(1);
  });

  it("keeps safe recovery through repeated busy rejections and never widens retry permissions", async () => {
    mocks.overlayApi.commit.mockRejectedValueOnce(new Error("reply channel lost"));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    const before = selectionRect();
    await act(async () => button("Save").click());
    const actions = () => [...document.querySelectorAll(".overlay-output-actions button")];
    const busy = { message: "截图会话正在处理中", outputPending: false, retryActions: [] };
    for (const failure of [busy, busy, { message: "copy unavailable", outputPending: true, retryActions: ["copy", "save", "pin"] }]) {
      mocks.overlayApi.retry.mockRejectedValueOnce(failure);
      await act(async () => actions()[0].click());
      expect(document.querySelector(".overlay-output-failure")).not.toBeNull();
      expect(actions().map((node) => node.textContent)).toEqual(["Copy", "Discard image"]);
      expect(selectionRect()).toEqual(before);
      expect(button("Save").disabled).toBe(true);
      expect(button("Pin").disabled).toBe(true);
    }
    await act(async () => actions()[0].click());
    expect(mocks.overlayApi.retry).toHaveBeenCalledTimes(4);
    expect(mocks.overlayApi.retry.mock.calls.every(([action]) => action === "copy")).toBe(true);
    expect(mocks.overlayApi.commit).toHaveBeenCalledTimes(1);
  });

  it("keeps the selection on right-click during output and restores reset only after confirmed render failure", async () => {
    let reject;
    mocks.overlayApi.commit.mockReturnValueOnce(new Promise((_, fail) => (reject = fail)));
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    const before = selectionRect();
    await act(async () => button("Save").click());
    const rightClick = () => document.querySelector(".overlay-root").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await act(async () => { expect(rightClick()).toBe(false); });
    expect(selectionRect()).toEqual(before);
    await act(async () => reject({ message: "render failed", outputPending: false, retryActions: [] }));
    expect(selectionRect()).toEqual(before);
    expect(button("Save").disabled).toBe(false);
    await act(async () => { rightClick(); });
    expect(selectionRect()).toBeNull();
  });

  it("keeps the selection on right-click while a failed artifact is retained", async () => {
    mocks.overlayApi.commit.mockRejectedValueOnce({ message: "disk full", outputPending: true, retryActions: ["copy", "save", "pin"] });
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    const before = selectionRect();
    await act(async () => button("Save").click());
    await act(async () => document.querySelector(".overlay-root").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true })));
    expect(selectionRect()).toEqual(before);
    expect(document.querySelector(".overlay-output-failure")).not.toBeNull();
  });

  it("rejects too-short physical Longshot selection and unlocks after later activation failure", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 20 });
    expect(button("Long screenshot").disabled).toBe(true);
    await drag({ x: 20, y: 30 }, { x: 120, y: 120 });
    await act(async () => button("Long screenshot").click());
    expect(button("Long screenshot").disabled).toBe(true);
    const handoff = mocks.overlayApi.onHandoff.mock.calls.at(-1)[0];
    await act(async () => handoff({ sessionId: "session-1", controllerLabel: "longshot-controller-session-1", accepted: false }));
    expect(button("Long screenshot").disabled).toBe(false);
    await act(async () => button("Long screenshot").click());
    expect(mocks.overlayApi.openLongshot).toHaveBeenCalledTimes(2);
  });

  it("keeps an activation failure that arrives before the open response", async () => {
    await mount();
    await drag({ x: 10, y: 10 }, { x: 110, y: 90 });
    let resolve;
    mocks.overlayApi.openLongshot.mockReturnValue(new Promise((done) => (resolve = done)));
    await act(async () => button("Long screenshot").click());
    const handoff = mocks.overlayApi.onHandoff.mock.calls.at(-1)[0];
    await act(async () => handoff({ sessionId: "session-1", controllerLabel: "new-controller", accepted: false }));
    await act(async () => resolve({ label: "new-controller" }));
    expect(button("Long screenshot").disabled).toBe(false);
    await act(async () => handoff({ sessionId: "another-session", controllerLabel: "new-controller", accepted: true }));
    expect(button("Long screenshot").disabled).toBe(false);
  });

});
