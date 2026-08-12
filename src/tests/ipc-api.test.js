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
  closeCurrentWindow,
  copyScreenshotImage,
  disableAutostart,
  enableAutostart,
  getClips,
  getCurrentWindowLabel,
  hideCurrentWindow,
  isAutostartEnabled,
  onClipAdded,
  runCaptureAction,
  startDraggingCurrentWindow,
  updateConfig,
  updatePin,
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

  it("sends AppConfig through the stable newConfig argument", () => {
    const config = {
      version: 1,
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
      translation_provider: "libretranslate",
      translation_endpoint: "https://libretranslate.com",
      translation_model: "",
      translation_source_language: "auto",
      translation_target_language: "en",
    };

    updateConfig(config);
    expect(invoke).toHaveBeenCalledWith("update_config", { newConfig: config });
  });

  it("preserves capture action and sessionId contracts", () => {
    const selection = {
      sessionId: "capture-7",
      monitorId: 2,
      x: 10,
      y: 20,
      width: 300,
      height: 180,
    };

    runCaptureAction("pin", selection);
    cancelCaptureOverlay(selection.sessionId);

    expect(invoke).toHaveBeenNthCalledWith(1, "run_capture_action", {
      action: "pin",
      selection,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "cancel_capture_overlay", {
      sessionId: "capture-7",
    });
  });

  it("preserves pngBase64 and nested pin update names", () => {
    copyScreenshotImage("encoded-png");
    updatePin("pin-image-1", { scale: 1.5, opacity: 0.8, locked: true });

    expect(invoke).toHaveBeenNthCalledWith(1, "copy_screenshot_image", {
      pngBase64: "encoded-png",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "update_pin", {
      label: "pin-image-1",
      update: { scale: 1.5, opacity: 0.8, locked: true },
    });
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
