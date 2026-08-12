import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createPastePermissionController,
  describePasteStatus,
} from "../js/settings/paste-permission.js";
import {
  closeAfterShortcutCleanup,
  createShortcutRecordingController,
} from "../js/settings/shortcut-recording.js";
import { formatByteSize } from "../js/settings/stats.js";
import { createThemePicker } from "../js/settings/theme-picker.js";

const translate = (key) => ({
  "settings.shortcut.record": "Record",
  "settings.shortcut.recording": "Recording...",
  "settings.shortcut.stop": "Stop",
}[key] || key);

function createRecorder(name, options = {}) {
  const input = document.createElement("input");
  const recordButton = document.createElement("button");
  const clearButton = document.createElement("button");
  const warning = document.createElement("div");
  warning.classList.add("hidden");
  document.body.append(input, recordButton, clearButton, warning);
  return {
    input,
    recordButton,
    clearButton,
    warning,
    defaultValue: options.defaultValue || "",
    getSavedValue: () => options.savedValue || `${name}-saved`,
    checkConflict: options.checkConflict,
  };
}

describe("settings paste permission status", () => {
  it.each([
    [{ backend: "x11", phase: "ready" }, "settings.autoPaste.x11Ready", "ready", false],
    [
      { backend: "wayland_portal", phase: "ready" },
      "settings.autoPaste.portalReady",
      "ready",
      false,
    ],
    [
      { backend: "wayland_portal", phase: "initializing" },
      "settings.autoPaste.initializing",
      "pending",
      true,
    ],
    [
      { backend: "wayland_portal", phase: "denied" },
      "settings.autoPaste.denied",
      "unavailable",
      true,
    ],
    [
      { backend: "wayland_portal", phase: "permission_required" },
      "settings.autoPaste.permissionRequired",
      "pending",
      true,
    ],
    [
      { backend: "copy_only", phase: "unavailable" },
      "settings.autoPaste.unavailable",
      "unavailable",
      false,
    ],
  ])("maps %# without platform-specific DOM branching", (status, i18nKey, tone, showAuthorize) => {
    expect(describePasteStatus(status)).toEqual({ i18nKey, tone, showAuthorize });
  });

  it("renders request failures and restores the authorize button", async () => {
    const statusDot = document.createElement("span");
    const statusText = document.createElement("span");
    const authorizeButton = document.createElement("button");
    const controller = createPastePermissionController({
      statusDot,
      statusText,
      authorizeButton,
      getStatus: vi.fn(),
      requestPermission: vi.fn().mockRejectedValue(new Error("denied by portal")),
      translate,
    });

    authorizeButton.click();
    await vi.waitFor(() => expect(authorizeButton.disabled).toBe(false));

    expect(statusDot.className).toBe("permission-status-dot unavailable");
    expect(statusText.textContent).toBe("settings.autoPaste.denied");
    expect(statusText.title).toContain("denied by portal");
    expect(authorizeButton.hidden).toBe(false);
    controller.refreshLabels();
    expect(statusText.textContent).toBe("settings.autoPaste.denied");
  });
});

describe("settings theme picker", () => {
  beforeEach(() => {
    document.body.replaceChildren();
    delete document.documentElement.dataset.theme;
  });

  it("renders static preview nodes and persists a selected theme", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const persistTheme = vi.fn().mockResolvedValue(undefined);
    const picker = createThemePicker({ container, translate, persistTheme });

    picker.initialize("dark");
    expect(container.querySelectorAll(".theme-card")).toHaveLength(6);
    expect(container.querySelectorAll(".theme-preview .tp-row")).toHaveLength(18);
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(container.querySelector('[data-theme="dark"][role="radio"]')?.ariaChecked).toBe("true");

    container.querySelector('[data-theme="rose"][role="radio"]').click();
    await vi.waitFor(() => expect(persistTheme).toHaveBeenCalledWith("rose"));
    expect(picker.value).toBe("rose");
    expect(document.documentElement.dataset.theme).toBe("rose");
  });
});

describe("settings shortcut recording controller", () => {
  beforeEach(() => {
    document.body.replaceChildren();
  });

  it("keeps recorders mutually exclusive and resumes shortcuts between them", async () => {
    const pauseShortcuts = vi.fn().mockResolvedValue(undefined);
    const resumeShortcuts = vi.fn().mockResolvedValue(undefined);
    const global = createRecorder("global");
    const pin = createRecorder("pin");
    const controller = createShortcutRecordingController({
      recorders: { global, pin },
      pauseShortcuts,
      resumeShortcuts,
      translate,
    });

    global.recordButton.click();
    await vi.waitFor(() => expect(controller.activeKey).toBe("global"));
    expect(global.input.classList.contains("recording")).toBe(true);

    pin.recordButton.click();
    await vi.waitFor(() => expect(controller.activeKey).toBe("pin"));
    expect(global.input.classList.contains("recording")).toBe(false);
    expect(pin.input.classList.contains("recording")).toBe(true);
    expect(pauseShortcuts).toHaveBeenCalledTimes(2);
    expect(resumeShortcuts).toHaveBeenCalledOnce();

    await controller.stop();
  });

  it("records a shortcut, checks conflicts, and defers listener cleanup", async () => {
    const deferred = [];
    const checkConflict = vi.fn().mockResolvedValue(true);
    const global = createRecorder("global", { checkConflict });
    const controller = createShortcutRecordingController({
      recorders: { global },
      pauseShortcuts: vi.fn().mockResolvedValue(undefined),
      resumeShortcuts: vi.fn().mockResolvedValue(undefined),
      translate,
      defer: (callback) => deferred.push(callback),
    });

    global.recordButton.click();
    await vi.waitFor(() => expect(controller.activeKey).toBe("global"));
    window.dispatchEvent(new KeyboardEvent("keydown", {
      code: "KeyV",
      key: "v",
      ctrlKey: true,
      altKey: true,
      bubbles: true,
      cancelable: true,
    }));

    await vi.waitFor(() => expect(checkConflict).toHaveBeenCalledWith("Ctrl+Alt+V"));
    expect(global.input.value).toBe("Ctrl+Alt+V");
    expect(global.warning.classList.contains("hidden")).toBe(false);
    expect(deferred).toHaveLength(1);
    deferred[0]();
    await vi.waitFor(() => expect(controller.activeKey).toBeNull());
  });

  it("waits for a pending pause before resuming shortcuts", async () => {
    let resolvePause;
    const pauseShortcuts = vi.fn(
      () => new Promise((resolve) => {
        resolvePause = resolve;
      }),
    );
    const resumeShortcuts = vi.fn().mockResolvedValue(undefined);
    const global = createRecorder("global");
    const controller = createShortcutRecordingController({
      recorders: { global },
      pauseShortcuts,
      resumeShortcuts,
      translate,
    });

    global.recordButton.click();
    await vi.waitFor(() => expect(pauseShortcuts).toHaveBeenCalledOnce());
    const stopped = controller.stop();
    await Promise.resolve();
    expect(resumeShortcuts).not.toHaveBeenCalled();

    resolvePause();
    await stopped;
    expect(resumeShortcuts).toHaveBeenCalledOnce();
    expect(controller.activeKey).toBeNull();
  });

  it("restores shortcuts before closing while recording", async () => {
    const order = [];
    const global = createRecorder("global");
    const controller = createShortcutRecordingController({
      recorders: { global },
      pauseShortcuts: vi.fn().mockResolvedValue(undefined),
      resumeShortcuts: vi.fn(async () => order.push("resume")),
      translate,
    });
    const closeWindow = vi.fn(async () => order.push("close"));

    global.recordButton.click();
    await vi.waitFor(() => expect(controller.activeKey).toBe("global"));
    await closeAfterShortcutCleanup(controller, closeWindow);

    expect(order).toEqual(["resume", "close"]);
    expect(closeWindow).toHaveBeenCalledOnce();
  });
});

describe("settings statistics formatting", () => {
  it.each([
    [0, "0 B"],
    [1023, "1023 B"],
    [1536, "1.5 KB"],
    [2 * 1024 * 1024, "2.0 MB"],
  ])("formats %i bytes as %s", (bytes, expected) => {
    expect(formatByteSize(bytes)).toBe(expected);
  });
});
