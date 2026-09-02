import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  invoke,
  listen,
  currentWindow,
  enableAutostartPlugin,
  disableAutostartPlugin,
  isAutostartEnabledPlugin,
} = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  currentWindow: {
    label: "settings",
    close: vi.fn(),
    hide: vi.fn(),
    startDragging: vi.fn(),
  },
  enableAutostartPlugin: vi.fn(),
  disableAutostartPlugin: vi.fn(),
  isAutostartEnabledPlugin: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => currentWindow,
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  enable: enableAutostartPlugin,
  disable: disableAutostartPlugin,
  isEnabled: isAutostartEnabledPlugin,
}));

import {
  cancelCaptureOverlay,
  markCaptureOverlayReady,
  copyText,
  closeCurrentWindow,
  disableAutostart,
  enableAutostart,
  getClips,
  getCurrentWindowLabel,
  hideCurrentWindow,
  isAutostartEnabled,
  onClipAdded,
  pickScreenshotDirectory,
  runCaptureDiagnostics,
  commitCaptureAction,
  startDraggingCurrentWindow,
  updateConfig,
  updatePin,
  copyPinCanvas,
  openPinImageDialog,
  savePinCanvas,
} from "../js/api.ts";

describe("typed IPC wrappers", () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    currentWindow.close.mockReset();
    currentWindow.hide.mockReset();
    currentWindow.startDragging.mockReset();
    enableAutostartPlugin.mockReset();
    disableAutostartPlugin.mockReset();
    isAutostartEnabledPlugin.mockReset();
  });

  it("keeps camelCase query arguments for get_clips", () => {
    getClips("needle", true, 8, 40);

    expect(invoke).toHaveBeenCalledWith("get_clips", {
      query: "needle",
      favoritesOnly: true,
      offset: 8,
      limit: 40,
    });
  });

  it("uses the explicit text-copy command without paste side effects", () => {
    copyText("translated result");
    expect(invoke).toHaveBeenCalledWith("copy_text", { text: "translated result" });
  });

  it("keeps editable/flat canvas save and current-composition copy contracts explicit", () => {
    const project = {
      rendererVersion: 1,
      sourceWidth: 320,
      sourceHeight: 180,
      annotations: [],
      adjustments: {},
    };
    savePinCanvas("pin-1", "png", true, "editable", project);
    copyPinCanvas("pin-1", "composed");
    openPinImageDialog();

    expect(invoke).toHaveBeenNthCalledWith(1, "save_pin_canvas", {
      label: "pin-1",
      pngBase64: "png",
      toClipboard: true,
      mode: "editable",
      project,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "copy_pin_canvas", {
      label: "pin-1",
      pngBase64: "composed",
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "open_pin_image_dialog");
  });

  it("sends AppConfig through the stable newConfig argument", () => {
    const config = {
      version: 2,
      max_history: 100,
      storage_mode: "persistent",
      global_shortcut: "Alt+V",
      pin_shortcut: "Ctrl+2",
      capture_shortcut: "Ctrl+Shift+S",
      theme: "light",
      language: "auto",
      delete_confirm_ms: 1200,
      ocr_result_mode: "preview",
      ocr_enabled: true,
      tmux_capture: false,
      auto_paste: true,
      translation_services: [
        {
          provider: "libretranslate",
          enabled: true,
          endpoint: "",
          model: "",
          region: "",
          project: "",
        },
      ],
      translation_source_language: "auto",
      translation_target_language: "en",
    };

    updateConfig(config);
    expect(invoke).toHaveBeenCalledWith("update_config", { newConfig: config });
  });

  /**
   * 覆盖层自己完成裁剪与标注，提交时送的是渲染好的 PNG——后端不再收选区，
   * 否则画布上的标注会被丢掉。这四个字段名是合同，改名就会静默失败。
   * `origin` 是选区在桌面逻辑坐标里的矩形，贴图靠它回到原位；省略即 null。
   */
  it("preserves the commit contract and sessionId names", () => {
    const origin = { x: 120, y: 48, width: 640, height: 360 };
    commitCaptureAction("pin", "capture-7", "encoded-png", origin);
    cancelCaptureOverlay("capture-7");

    expect(invoke).toHaveBeenNthCalledWith(1, "commit_capture_action", {
      action: "pin",
      sessionId: "capture-7",
      pngBase64: "encoded-png",
      origin,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "cancel_capture_overlay", {
      sessionId: "capture-7",
    });
  });

  /** 不知道来源的图片（不是从截图选区来的）必须显式传 null，后端据此落回默认摆放。 */
  it("sends a null origin when the caller does not know where the image came from", () => {
    commitCaptureAction("copy", "capture-8", "encoded-png");
    expect(invoke).toHaveBeenCalledWith("commit_capture_action", {
      action: "copy",
      sessionId: "capture-8",
      pngBase64: "encoded-png",
      origin: null,
    });
  });

  /**
   * 覆盖层隐藏建窗，显示时机由前端报告首帧决定；参数名改了就会一直白屏/不显示。
   * 实测视口跟着这次握手一起走（不变量 I4），少了它多屏几何算错时就没人发现。
   */
  it("preserves the overlay reveal handshake contract", () => {
    markCaptureOverlayReady("capture-overlay-7-0", 1920, 1200);
    expect(invoke).toHaveBeenCalledWith("mark_capture_overlay_ready", {
      label: "capture-overlay-7-0",
      viewportWidth: 1920,
      viewportHeight: 1200,
    });
  });

  it("sends a null note when the user did not describe the symptom", () => {
    runCaptureDiagnostics();
    expect(invoke).toHaveBeenCalledWith("run_capture_diagnostics", { note: null });

    runCaptureDiagnostics("外接屏拔掉后覆盖层还是双屏大小");
    expect(invoke).toHaveBeenLastCalledWith("run_capture_diagnostics", {
      note: "外接屏拔掉后覆盖层还是双屏大小",
    });
  });

  it("preserves nested pin update names", () => {
    updatePin("pin-image-1", { scale: 1.5, opacity: 0.8, locked: true });

    expect(invoke).toHaveBeenCalledWith("update_pin", {
      label: "pin-image-1",
      update: { scale: 1.5, opacity: 0.8, locked: true },
    });
  });

  it("passes a cancelled directory dialog through as null", async () => {
    invoke.mockResolvedValueOnce(null);

    await expect(pickScreenshotDirectory()).resolves.toBeNull();
    expect(invoke).toHaveBeenNthCalledWith(1, "pick_screenshot_directory");
  });

  it("delivers typed event payloads without exposing the Tauri envelope", async () => {
    const unlisten = vi.fn();
    let listener;
    listen.mockImplementation((_event, callback) => {
      listener = callback;
      return Promise.resolve(unlisten);
    });
    const callback = vi.fn();
    const clip = { id: 5, content_type: "text", byte_size: 4 };

    await expect(onClipAdded(callback)).resolves.toBe(unlisten);
    listener({ payload: clip });

    expect(listen).toHaveBeenCalledWith("clip-added", expect.any(Function));
    expect(callback).toHaveBeenCalledWith(clip);
  });

  it("keeps current-window access behind the typed boundary", () => {
    expect(getCurrentWindowLabel()).toBe("settings");

    closeCurrentWindow();
    hideCurrentWindow();
    startDraggingCurrentWindow();

    expect(currentWindow.close).toHaveBeenCalledOnce();
    expect(currentWindow.hide).toHaveBeenCalledOnce();
    expect(currentWindow.startDragging).toHaveBeenCalledOnce();
  });

  it("keeps autostart plugin access behind the typed boundary", () => {
    enableAutostart();
    disableAutostart();
    isAutostartEnabled();

    expect(enableAutostartPlugin).toHaveBeenCalledOnce();
    expect(disableAutostartPlugin).toHaveBeenCalledOnce();
    expect(isAutostartEnabledPlugin).toHaveBeenCalledOnce();
  });
});
