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
    onDragDropEvent: vi.fn(),
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
  cancelLongshotController,
  activateLongshotController,
  appendLongshotController,
  undoLongshotController,
  previewLongshotController,
  finishLongshotController,
  markCaptureOverlayReady,
  markLongshotControllerReady,
  copyText,
  detectImageCodes,
  closeCurrentWindow,
  disableAutostart,
  enableAutostart,
  getClips,
  getCurrentWindowLabel,
  getOcrHealthStatus,
  hideCurrentWindow,
  isAutostartEnabled,
  onClipAdded,
  onMainWindowWillHide,
  onPasteFallback,
  pickScreenshotDirectory,
  pickOcrManifest,
  runCaptureDiagnostics,
  commitCaptureAction,
  retryCaptureAction,
  onCaptureLongshotHandoff,
  startDraggingCurrentWindow,
  updateConfig,
  closeSettings,
  restartApp,
  checkUpdate,
  getAppUpdateState,
  onAppUpdateState,
  downloadAndInstallUpdate,
  updatePin,
  copyPinCanvas,
  onCurrentWindowDragDrop,
  openPinProjectFile,
  openLongshotController,
  savePinCanvas,
} from "../js/api.ts";

describe("typed IPC wrappers", () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    currentWindow.close.mockReset();
    currentWindow.hide.mockReset();
    currentWindow.startDragging.mockReset();
    currentWindow.onDragDropEvent.mockReset();
    enableAutostartPlugin.mockReset();
    disableAutostartPlugin.mockReset();
    isAutostartEnabledPlugin.mockReset();
  });

  it("updates use process commands and absolute state events without plugin resources", async () => {
    const snapshot = { revision: 4, status: "installed", version: "2.0.0" };
    invoke.mockResolvedValue(snapshot);
    await expect(checkUpdate()).resolves.toBe(snapshot);
    expect(invoke).toHaveBeenLastCalledWith("check_app_update");
    await expect(getAppUpdateState()).resolves.toBe(snapshot);
    expect(invoke).toHaveBeenLastCalledWith("get_app_update_state");
    await expect(downloadAndInstallUpdate("2.0.0")).resolves.toBe(snapshot);
    expect(invoke).toHaveBeenLastCalledWith("install_app_update", { version: "2.0.0" });
    const callback = vi.fn(); await onAppUpdateState(callback);
    expect(listen).toHaveBeenCalledWith("app-update-state", expect.any(Function));
    listen.mock.calls[0][1]({ payload: snapshot });
    expect(callback).toHaveBeenCalledWith(snapshot);
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

  it("validates the bounded local image-code response before exposing it", async () => {
    const response = {
      results: [{
        format: "qr_code",
        text: "https://example.test/\u0000\u202E",
        points: [{ x: 0, y: 1 }, { x: 16_383, y: 16_383 }],
      }],
      limited: false,
    };
    invoke.mockResolvedValueOnce(response);

    await expect(detectImageCodes(42)).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("detect_image_codes", { id: 42 });
  });

  it("rejects malformed, oversized, or unsupported local image-code responses", async () => {
    const oversizedText = "测".repeat(5_462); // UTF-8 超过 16 KiB，不能按 JS length 判断。
    const boundedText = "x".repeat(16 * 1024);
    const result = { format: "qr_code", text: "x", points: [] };
    const invalidResponses = [
      null,
      [],
      { results: Array.from({ length: 33 }, () => result), limited: false },
      { results: [], limited: "false" },
      { results: [{ format: "aztec", text: "x", points: [] }], limited: false },
      { results: [{ format: "qr_code", text: oversizedText, points: [] }], limited: false },
      {
        results: Array.from(
          { length: 5 },
          () => ({ format: "qr_code", text: boundedText, points: [] }),
        ),
        limited: false,
      },
      {
        results: [{
          format: "qr_code",
          text: "x",
          points: Array.from({ length: 65 }, () => ({ x: 0, y: 0 })),
        }],
        limited: false,
      },
      { results: [{ format: "code_39", text: "x", points: [{ x: -1, y: 0 }] }], limited: false },
      { results: [{ format: "code_39", text: "x", points: [{ x: Infinity, y: 0 }] }], limited: false },
      { results: [{ format: "code_39", text: "x", points: [{ x: 16_385, y: 0 }] }], limited: false },
      { results: [{ format: "code_39", text: "x", points: [null] }], limited: false },
      { results: [], limited: false, unexpected: true },
    ];

    for (const response of invalidResponses) {
      invoke.mockResolvedValueOnce(response);
      await expect(detectImageCodes(43)).rejects.toThrow("invalid image code scan");
    }
  });

  it("keeps editable/flat canvas save and current-composition copy contracts explicit", () => {
    const project = {
      rendererVersion: 2,
      sourceWidth: 320,
      sourceHeight: 180,
      annotations: [],
      adjustments: {},
    };
    savePinCanvas("pin-1", null, true, "editable", project);
    savePinCanvas("pin-1", null, false, "flat", null);
    copyPinCanvas("pin-1", project);

    expect(invoke).toHaveBeenNthCalledWith(1, "save_pin_canvas", {
      label: "pin-1",
      pngBase64: null,
      toClipboard: true,
      mode: "editable",
      project,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "save_pin_canvas", {
      label: "pin-1",
      pngBase64: null,
      toClipboard: false,
      mode: "flat",
      project: null,
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "copy_pin_canvas", {
      label: "pin-1",
      pngBase64: null,
      project,
    });
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

  /** 选区和 v2 操作层是合同；后端从会话冻结帧生成权威 PNG。 */
  it("preserves the commit contract and sessionId names", () => {
    const origin = { x: 120, y: 48, width: 640, height: 360 };
    const selection = { sessionId: "capture-7", monitorId: 2, x: 4, y: 5, width: 640, height: 360 };
    const project = {
      rendererVersion: 2,
      sourceWidth: 1920,
      sourceHeight: 1080,
      annotations: [],
      adjustments: { grayscale: false, brightness: 0, contrast: 0, saturation: 0, cornerRadius: 0 },
    };
    commitCaptureAction("pin", selection, project, origin);
    cancelCaptureOverlay("capture-7");

    expect(invoke).toHaveBeenNthCalledWith(1, "commit_capture_action", {
      action: "pin",
      selection,
      project,
      origin,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "cancel_capture_overlay", {
      sessionId: "capture-7",
    });
  });

  /** 不知道来源的图片（不是从截图选区来的）必须显式传 null，后端据此落回默认摆放。 */
  it("sends a null origin when the caller does not know where the image came from", () => {
    const selection = { sessionId: "capture-8", monitorId: 0, x: 0, y: 0, width: 10, height: 10 };
    const project = {
      rendererVersion: 2,
      sourceWidth: 10,
      sourceHeight: 10,
      annotations: [],
      adjustments: { grayscale: false, brightness: 0, contrast: 0, saturation: 0, cornerRadius: 0 },
    };
    commitCaptureAction("copy", selection, project);
    expect(invoke).toHaveBeenCalledWith("commit_capture_action", {
      action: "copy",
      selection,
      project,
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

  it("retries the retained capture using the IPC caller and forwards handoff identities", () => {
    retryCaptureAction("save");
    expect(invoke).toHaveBeenCalledWith("retry_capture_action", { action: "save" });
    const callback = vi.fn();
    onCaptureLongshotHandoff(callback);
    const [name, receive] = listen.mock.calls[0];
    expect(name).toBe("capture-longshot-handoff");
    const payload = { controllerLabel: "controller-1", sessionId: "session-1", accepted: false };
    receive({ payload });
    expect(callback).toHaveBeenCalledWith(payload);
  });

  it("keeps the longshot controller wire contract label-free and lossless", () => {
    const selection = {
      sessionId: "capture-7",
      monitorId: 2,
      x: 4,
      y: 5,
      width: 640,
      height: 360,
    };
    const handle = { sessionId: "longshot-7", generation: "18446744073709551615" };

    openLongshotController(selection);
    activateLongshotController();
    markLongshotControllerReady();
    appendLongshotController(handle);
    undoLongshotController(handle);
    finishLongshotController(handle, "copy");
    finishLongshotController(handle, "save");
    finishLongshotController(handle, "pin");
    cancelLongshotController(handle);
    cancelLongshotController(null);

    expect(invoke).toHaveBeenNthCalledWith(1, "open_longshot_controller", { selection });
    expect(invoke).toHaveBeenNthCalledWith(2, "activate_longshot_controller");
    expect(invoke).toHaveBeenNthCalledWith(3, "mark_longshot_controller_ready");
    expect(invoke).toHaveBeenNthCalledWith(4, "append_longshot_controller", { handle });
    expect(invoke).toHaveBeenNthCalledWith(5, "undo_longshot_controller", { handle });
    expect(invoke).toHaveBeenNthCalledWith(6, "finish_longshot_controller", {
      handle,
      action: "copy",
    });
    expect(invoke).toHaveBeenNthCalledWith(7, "finish_longshot_controller", {
      handle,
      action: "save",
    });
    expect(invoke).toHaveBeenNthCalledWith(8, "finish_longshot_controller", {
      handle,
      action: "pin",
    });
    expect(invoke).toHaveBeenNthCalledWith(9, "cancel_longshot_controller", { handle });
    expect(invoke).toHaveBeenNthCalledWith(10, "cancel_longshot_controller", { handle: null });
  });

  it("passes complete longshot Copy, Save, and Pin result contracts through unchanged", async () => {
    const handle = { sessionId: "longshot-8", generation: "42" };
    const copied = { action: "copy", path: null, pinLabel: null };
    const saved = { action: "save", path: "/tmp/长截图.png", pinLabel: null };
    const pinned = { action: "pin", path: null, pinLabel: "pin-image-9" };
    invoke.mockResolvedValueOnce(copied).mockResolvedValueOnce(saved).mockResolvedValueOnce(pinned);

    await expect(finishLongshotController(handle, "copy")).resolves.toBe(copied);
    await expect(finishLongshotController(handle, "save")).resolves.toBe(saved);
    await expect(finishLongshotController(handle, "pin")).resolves.toBe(pinned);
    expect(invoke).toHaveBeenNthCalledWith(1, "finish_longshot_controller", {
      handle,
      action: "copy",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "finish_longshot_controller", {
      handle,
      action: "save",
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "finish_longshot_controller", {
      handle,
      action: "pin",
    });
  });

  it("accepts only bounded non-empty ArrayBuffer longshot previews with the exact handle", async () => {
    const handle = { sessionId: "longshot-preview-8", generation: "43" };
    const valid = new Uint8Array([137, 80, 78, 71]).buffer;
    invoke.mockResolvedValueOnce(valid);

    await expect(previewLongshotController(handle)).resolves.toBe(valid);
    expect(invoke).toHaveBeenCalledWith("preview_longshot_controller", { handle });
  });

  it.each([
    ["empty", () => new ArrayBuffer(0)],
    ["over 1 MiB", () => new ArrayBuffer(1024 * 1024 + 1)],
    ["array", () => [137, 80, 78, 71]],
    ["string", () => "png"],
    ["plain object", () => ({ byteLength: 4 })],
    ["SharedArrayBuffer", () => new SharedArrayBuffer(4)],
    ["proxied ArrayBuffer", () => new Proxy(new ArrayBuffer(4), {})],
    ["detached ArrayBuffer", () => {
      const detached = new ArrayBuffer(4);
      structuredClone(detached, { transfer: [detached] });
      return detached;
    }],
  ])("rejects a %s longshot preview response", async (_name, value) => {
    const handle = { sessionId: "longshot-preview-invalid", generation: "44" };
    invoke.mockResolvedValueOnce(value());

    await expect(previewLongshotController(handle)).rejects.toThrow("invalid longshot preview response");
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

  it("keeps enhanced OCR health and manifest picking behind settings IPC", async () => {
    invoke.mockResolvedValueOnce({ activeEngine: "tesseract" });
    await getOcrHealthStatus("/opt/clippy-ocr/manifest.json");
    expect(invoke).toHaveBeenLastCalledWith("ocr_health_status", {
      manifestPath: "/opt/clippy-ocr/manifest.json",
    });

    invoke.mockResolvedValueOnce("/opt/clippy-ocr/manifest.json");
    await expect(pickOcrManifest()).resolves.toBe("/opt/clippy-ocr/manifest.json");
    expect(invoke).toHaveBeenLastCalledWith("pick_ocr_manifest");
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

  it("delivers structured automatic-paste fallback reasons", async () => {
    let listener;
    listen.mockImplementation((_event, callback) => {
      listener = callback;
      return Promise.resolve(vi.fn());
    });
    const callback = vi.fn();
    const outcome = {
      copied: true,
      pasted: false,
      backend: "windows_send_input",
      reason_code: "windows_integrity_boundary",
      detail: "blocked by UIPI",
    };

    await onPasteFallback(callback);
    listener({ payload: outcome });

    expect(listen).toHaveBeenCalledWith("paste-fallback", expect.any(Function));
    expect(callback).toHaveBeenCalledWith(outcome);
  });

  it("delivers explicit main-window hide notifications", async () => {
    let listener;
    listen.mockImplementation((_event, callback) => {
      listener = callback;
      return Promise.resolve(vi.fn());
    });
    const callback = vi.fn();

    await onMainWindowWillHide(callback);
    listener({ payload: null });

    expect(listen).toHaveBeenCalledWith("main-window-will-hide", expect.any(Function));
    expect(callback).toHaveBeenCalledOnce();
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

  it("keeps native project drop and project-open IPC behind the typed boundary", async () => {
    const unlisten = vi.fn();
    const callback = vi.fn();
    let nativeCallback;
    currentWindow.onDragDropEvent.mockImplementation((handler) => {
      nativeCallback = handler;
      return Promise.resolve(unlisten);
    });

    await expect(onCurrentWindowDragDrop(callback)).resolves.toBe(unlisten);
    openPinProjectFile("/tmp/annotated.png");
    const payload = { type: "drop", paths: ["/tmp/annotated.png"], position: { x: 0, y: 0 } };
    nativeCallback({ payload });

    expect(currentWindow.onDragDropEvent).toHaveBeenCalledWith(expect.any(Function));
    expect(callback).toHaveBeenCalledWith(payload);
    expect(invoke).toHaveBeenCalledWith("open_pin_project_file", { path: "/tmp/annotated.png" });
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


it("只在显式调用时请求重启 IPC，并返回配置的 pending 状态", async () => {
  invoke.mockResolvedValueOnce({ shortcut_status: "pending" });
  await expect(updateConfig({ theme: "dark" })).resolves.toEqual({ shortcut_status: "pending" });
  expect(invoke).toHaveBeenLastCalledWith("update_config", { newConfig: { theme: "dark" } });
  invoke.mockResolvedValueOnce(undefined);
  await restartApp();
  expect(invoke).toHaveBeenLastCalledWith("restart_app");
});

it("显式 Save 要求快捷键完成恢复，关闭使用后端恢复守卫", async () => {
  await updateConfig({ theme: "dark" }, { requireShortcutsActive: true });
  expect(invoke).toHaveBeenLastCalledWith("update_config", { newConfig: { theme: "dark" }, requireShortcutsActive: true });
  await closeSettings();
  expect(invoke).toHaveBeenLastCalledWith("close_settings");
});
