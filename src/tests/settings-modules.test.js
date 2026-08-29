import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createPastePermissionController,
  describePasteStatus,
} from "../js/settings/paste-permission.js";
import {
  closeAfterShortcutCleanup,
  createShortcutRecordingController,
} from "../js/settings/shortcut-recording.js";
import { createShortcutFailureNotice } from "../js/settings/shortcut-failure-notice.js";
import { createScreenshotSettings } from "../js/settings/screenshot-settings.js";
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

  it("flags a combination already used by another Clippy action without asking the backend", async () => {
    const checkConflict = vi.fn().mockResolvedValue({ conflicted: false, source: null });
    const global = createRecorder("global", { checkConflict });
    const pin = createRecorder("pin", { checkConflict });
    pin.input.value = "ctrl+alt+v"; // 大小写与顺序都不该影响判断
    const controller = createShortcutRecordingController({
      recorders: { global, pin },
      pauseShortcuts: vi.fn().mockResolvedValue(undefined),
      resumeShortcuts: vi.fn().mockResolvedValue(undefined),
      translate,
      defer: () => {},
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

    await vi.waitFor(() =>
      expect(global.warning.classList.contains("hidden")).toBe(false));
    expect(global.warning.dataset.i18n).toBe("settings.shortcut.conflictSelf");
    expect(checkConflict).not.toHaveBeenCalled();
    await controller.stop();
  });

  it.each([
    [{ conflicted: true, source: "desktop", owner: "toggle-overview" }, "settings.shortcut.conflict"],
    [{ conflicted: true, source: "clippy", owner: null }, "settings.shortcut.conflictSelf"],
    [{ conflicted: true, source: "future-source" }, "settings.shortcut.conflict"],
  ])("renders the warning text that matches the reported conflict source %#", async (result, key) => {
    const checkConflict = vi.fn().mockResolvedValue(result);
    const global = createRecorder("global", { checkConflict });
    const controller = createShortcutRecordingController({
      recorders: { global },
      pauseShortcuts: vi.fn().mockResolvedValue(undefined),
      resumeShortcuts: vi.fn().mockResolvedValue(undefined),
      translate,
      defer: () => {},
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

    await vi.waitFor(() => expect(checkConflict).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(global.warning.dataset.i18n).toBe(key));
    expect(global.warning.classList.contains("hidden")).toBe(false);
    await controller.stop();
  });

  it("hides the warning when the backend cannot find a conflict", async () => {
    const checkConflict = vi.fn().mockResolvedValue({
      conflicted: false,
      source: null,
      owner: null,
      enumerable: false,
    });
    const global = createRecorder("global", { checkConflict });
    global.warning.classList.remove("hidden");
    const controller = createShortcutRecordingController({
      recorders: { global },
      pauseShortcuts: vi.fn().mockResolvedValue(undefined),
      resumeShortcuts: vi.fn().mockResolvedValue(undefined),
      translate,
      defer: () => {},
    });

    global.recordButton.click();
    await vi.waitFor(() => expect(controller.activeKey).toBe("global"));
    window.dispatchEvent(new KeyboardEvent("keydown", {
      code: "KeyV",
      key: "v",
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    }));

    await vi.waitFor(() => expect(checkConflict).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(global.warning.classList.contains("hidden")).toBe(true));
    await controller.stop();
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

describe("settings shortcut registration failures", () => {
  function mount(locale = "en") {
    const warning = document.createElement("div");
    warning.classList.add("hidden");
    document.body.replaceChildren(warning);
    const dictionary = {
      en: {
        "settings.shortcut.action.global": "Panel",
        "settings.shortcut.action.capture": "Screenshot",
        "settings.shortcut.registerFailed.wayland": "{action} shortcut ({shortcut}) needs a manual binding",
        "settings.shortcut.registerFailed.x11": "{action} shortcut ({shortcut}) is already taken",
      },
      "zh-CN": {
        "settings.shortcut.action.global": "面板",
        "settings.shortcut.registerFailed.wayland": "{action}快捷键（{shortcut}）需要手动绑定",
      },
    };
    let current = locale;
    const notice = createShortcutFailureNotice({
      warning,
      translate: (key, params) => {
        let text = dictionary[current][key] ?? key;
        for (const [name, value] of Object.entries(params ?? {})) {
          text = text.replace(`{${name}}`, String(value));
        }
        return text;
      },
    });
    return { warning, notice, setLocale: (next) => (current = next) };
  }

  it("shows the stored startup failures the frontend was not listening for", () => {
    const { warning, notice } = mount();

    notice.replaceAll([
      { action: "global", shortcut: "Super+V", session: "wayland", reason: "非 GNOME" },
      { action: "capture", shortcut: "Ctrl+Shift+S", session: "x11", reason: "already grabbed" },
    ]);

    expect(warning.classList.contains("hidden")).toBe(false);
    expect(warning.textContent.split("\n")).toEqual([
      "Panel shortcut (Super+V) needs a manual binding",
      "Screenshot shortcut (Ctrl+Shift+S) is already taken",
    ]);
  });

  it("keeps one line per action and clears when a later query reports success", () => {
    const { warning, notice } = mount();

    notice.add({ action: "global", shortcut: "Super+V", session: "wayland", reason: "" });
    notice.add({ action: "global", shortcut: "Alt+V", session: "wayland", reason: "" });
    expect(warning.textContent).toBe("Panel shortcut (Alt+V) needs a manual binding");

    notice.replaceAll([]);
    expect(warning.classList.contains("hidden")).toBe(true);
    expect(warning.textContent).toBe("");
  });

  it("re-renders interpolated text after a language switch", () => {
    const { warning, notice, setLocale } = mount();
    notice.add({ action: "global", shortcut: "Super+V", session: "wayland", reason: "" });

    setLocale("zh-CN");
    notice.refreshLabels();

    expect(warning.textContent).toBe("面板快捷键（Super+V）需要手动绑定");
  });
});

describe("settings screenshot save location", () => {
  function mount(pickDirectory) {
    const directoryInput = document.createElement("input");
    const browseButton = document.createElement("button");
    const templateInput = document.createElement("input");
    document.body.replaceChildren(directoryInput, browseButton, templateInput);
    const showToast = vi.fn();
    const controller = createScreenshotSettings({
      directoryInput,
      browseButton,
      templateInput,
      pickDirectory,
      translate,
      showToast,
    });
    return { controller, directoryInput, browseButton, templateInput, showToast };
  }

  it("keeps empty inputs empty so the backend default keeps applying", () => {
    const { controller, directoryInput, templateInput } = mount(vi.fn());

    controller.fill({ screenshot_save_dir: "", screenshot_filename_template: "" });
    expect(directoryInput.value).toBe("");
    expect(templateInput.value).toBe("");
    expect(controller.getConfig()).toEqual({
      screenshot_save_dir: "",
      screenshot_filename_template: "",
    });

    controller.fill({ screenshot_save_dir: "~/Shots", screenshot_filename_template: "cap-{date}" });
    // 输入里的空白不该写进配置，否则后端会当成一个真实目录名。
    directoryInput.value = "  ~/Other  ";
    templateInput.value = "  cap  ";
    expect(controller.getConfig()).toEqual({
      screenshot_save_dir: "~/Other",
      screenshot_filename_template: "cap",
    });
  });

  it("fills the picked folder and keeps the current one when the dialog is cancelled", async () => {
    const pickDirectory = vi.fn().mockResolvedValue("/home/user/Shots");
    const { directoryInput, browseButton } = mount(pickDirectory);

    browseButton.click();
    await vi.waitFor(() => expect(directoryInput.value).toBe("/home/user/Shots"));
    expect(browseButton.disabled).toBe(false);

    pickDirectory.mockResolvedValue(null);
    browseButton.click();
    await vi.waitFor(() => expect(pickDirectory).toHaveBeenCalledTimes(2));
    expect(directoryInput.value).toBe("/home/user/Shots");
  });

  it("reports a failed folder chooser and stays clickable", async () => {
    const pickDirectory = vi.fn().mockRejectedValue(new Error("no portal"));
    const { browseButton, directoryInput, showToast } = mount(pickDirectory);
    directoryInput.value = "~/Keep";

    browseButton.click();
    await vi.waitFor(() =>
      expect(showToast).toHaveBeenCalledWith("settings.screenshot.browseFailed"),
    );
    expect(directoryInput.value).toBe("~/Keep");
    expect(browseButton.disabled).toBe(false);
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
