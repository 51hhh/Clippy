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
import { initSettingsTabs } from "../js/settings/tabs.js";
import { createThemePicker } from "../js/settings/theme-picker.js";
import {
  createWindowProbeCard,
  describeWindowProbe,
} from "../js/settings/window-probe.js";
import {
  CAPTURE_ISSUE_URL,
  createCaptureDiagnosticsCard,
  describeReportPath,
} from "../js/settings/capture-diagnostics.js";
import {
  createPlatformCapabilities,
  describePlatformEnvironment,
  describePortalInterfaces,
} from "../js/settings/platform-capabilities.js";

const translate = (key) => ({
  "settings.shortcut.record": "Record",
  "settings.shortcut.recording": "Recording...",
  "settings.shortcut.stop": "Stop",
}[key] || key);

const platform = {
  operating_system: "linux",
  session: "wayland",
  desktop_environment: "kde",
  architecture: "x86_64",
  xwayland_available: true,
  portal: {
    desktop_service_available: true,
    global_shortcuts: { available: true, version: 2 },
    remote_desktop: { available: false, version: null },
    screenshot: { available: true, version: 1 },
    screen_cast: { available: false, version: null },
  },
  capabilities: {
    clipboard_text: { state: "available", reason: null },
    auto_paste: { state: "degraded", reason: "wayland_portal_unavailable" },
    global_shortcuts: { state: "permission_required", reason: "wayland_portal_permission" },
  },
};

describe("settings platform capabilities", () => {
  it("describes the backend facts without inferring from the browser", () => {
    expect(describePlatformEnvironment(platform, translate)).toContain("XWayland");
    expect(describePlatformEnvironment(platform, translate)).toContain("kde");
    expect(describePortalInterfaces(platform, translate)).toContain("GlobalShortcuts v2");
    expect(describePortalInterfaces(platform, translate)).toContain("RemoteDesktop —");
  });

  it("renders every capability with a translated state and reason using text nodes", () => {
    const summary = document.createElement("div");
    const portal = document.createElement("div");
    const list = document.createElement("div");
    const controller = createPlatformCapabilities({ summary, portal, list, translate });
    controller.render(platform);

    expect(list.children).toHaveLength(3);
    expect(list.children[0].classList.contains("ready")).toBe(true);
    expect(list.children[1].classList.contains("pending")).toBe(true);
    expect(list.textContent).toContain("settings.platform.reason.wayland_portal_unavailable");
    expect(list.querySelectorAll("script")).toHaveLength(0);
  });

  it("reports loading failure without leaving stale capability rows", () => {
    const summary = document.createElement("div");
    const portal = document.createElement("div");
    const list = document.createElement("div");
    const controller = createPlatformCapabilities({ summary, portal, list, translate });
    controller.render(platform);
    controller.renderError();
    expect(summary.textContent).toBe("settings.platform.loadFailed");
    expect(portal.textContent).toBe("");
    expect(list.children).toHaveLength(0);
  });
});

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
      { backend: "windows_send_input", phase: "ready" },
      "settings.autoPaste.windowsReady",
      "ready",
      false,
    ],
    [
      { backend: "macos_quartz", phase: "ready" },
      "settings.autoPaste.macosReady",
      "ready",
      false,
    ],
    [
      { backend: "macos_quartz", phase: "permission_required" },
      "settings.autoPaste.macosPermissionRequired",
      "pending",
      true,
    ],
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
        "settings.shortcut.registerFailed.native": "{action} shortcut ({shortcut}) was rejected by the OS",
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

  it("labels Windows and macOS failures as native instead of X11", () => {
    const { warning, notice } = mount();
    notice.replaceAll([
      { action: "global", shortcut: "Alt+V", session: "native", reason: "reserved" },
    ]);
    expect(warning.textContent).toBe("Panel shortcut (Alt+V) was rejected by the OS");
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
    return {
      controller,
      directoryInput,
      browseButton,
      templateInput,
      showToast,
    };
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

  // 提交动作的设置项随编辑器窗口一起删了：标注在覆盖层内完成，
  // 没有"转到编辑器"这个分支，配置里也不再有 capture_commit_action。
  it("no longer writes a commit action into the config", () => {
    const { controller } = mount(vi.fn());

    controller.fill({ capture_commit_action: "toolbar" });
    expect(controller.getConfig()).not.toHaveProperty("capture_commit_action");
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

describe("settings window quick-pick service", () => {
  const status = (overrides) => ({
    supported: true,
    installed: false,
    enabled: false,
    active: false,
    stale: false,
    userExtensionsEnabled: true,
    ...overrides,
  });

  it.each([
    [
      "非 GNOME 桌面不提这个服务",
      { supported: false },
      { tone: "neutral", showInstall: false, showUninstall: false },
    ],
    [
      "扩展在应答 D-Bus 就是唯一的可用判据",
      { installed: true, enabled: true, active: true },
      { tone: "ready", showInstall: false, showUninstall: true },
    ],
    [
      "升级后跑着的还是旧版扩展，不能说成已就绪",
      { installed: true, enabled: true, active: true, stale: true },
      {
        tone: "pending",
        stateKey: "settings.windowProbe.statePendingUpdate",
        showInstall: false,
        showUninstall: true,
      },
    ],
    [
      "系统层面关掉了全部扩展时再点安装没用",
      { installed: true, enabled: true, userExtensionsEnabled: false },
      { tone: "pending", showInstall: false, showUninstall: true },
    ],
    [
      "装好但还没注销",
      { installed: true, enabled: true },
      { tone: "pending", showInstall: false, showUninstall: true },
    ],
    ["没装", {}, { tone: "unavailable", showInstall: true, showUninstall: false }],
    [
      "目录没了但 gsettings 里还挂着条目，得留清理入口",
      { enabled: true },
      { tone: "unavailable", showInstall: true, showUninstall: true },
    ],
  ])("describes %s", (_name, overrides, expected) => {
    expect(describeWindowProbe(status(overrides))).toMatchObject(expected);
  });

  it("treats a missing status like an unsupported desktop instead of throwing", () => {
    expect(describeWindowProbe(undefined).tone).toBe("neutral");
  });

  function mount({ getStatus, install, uninstall }) {
    const card = document.createElement("div");
    const dot = document.createElement("span");
    const stateText = document.createElement("span");
    const detailText = document.createElement("p");
    const installButton = document.createElement("button");
    const uninstallButton = document.createElement("button");
    const recheckButton = document.createElement("button");
    document.body.replaceChildren(
      card,
      dot,
      stateText,
      detailText,
      installButton,
      uninstallButton,
      recheckButton,
    );
    const notify = vi.fn();
    const controller = createWindowProbeCard({
      card,
      dot,
      stateText,
      detailText,
      installButton,
      uninstallButton,
      recheckButton,
      getStatus,
      install,
      uninstall,
      translate,
      notify,
    });
    return {
      controller,
      card,
      dot,
      stateText,
      detailText,
      installButton,
      uninstallButton,
      recheckButton,
      notify,
    };
  }

  it("paints the tone onto the card and the dot", async () => {
    const mounted = mount({ getStatus: vi.fn().mockResolvedValue(status({ active: true })) });

    await mounted.controller.load();
    expect(mounted.card.className).toBe("service-card ready");
    expect(mounted.dot.className).toBe("service-card-dot ready");
    expect(mounted.stateText.textContent).toBe("settings.windowProbe.stateActive");
    expect(mounted.installButton.hidden).toBe(true);
    expect(mounted.uninstallButton.hidden).toBe(false);
  });

  // 装完必须注销一次才生效，这句提示不给就会被当成"装了没用"。
  it("tells the user to log out when the freshly installed extension is not answering yet", async () => {
    const mounted = mount({
      getStatus: vi.fn().mockResolvedValue(status()),
      install: vi.fn().mockResolvedValue({
        needsLogout: true,
        status: status({ installed: true, enabled: true }),
      }),
    });
    await mounted.controller.load();

    mounted.installButton.click();
    await vi.waitFor(() =>
      expect(mounted.notify).toHaveBeenCalledWith("settings.windowProbe.installedNeedsLogout"),
    );
    expect(mounted.card.className).toBe("service-card pending");
    expect(mounted.recheckButton.disabled).toBe(false);
  });

  it("reports a failed install, re-reads the real state and stays clickable", async () => {
    const getStatus = vi.fn().mockResolvedValue(status());
    const mounted = mount({
      getStatus,
      install: vi.fn().mockRejectedValue("写入扩展目录失败"),
    });
    await mounted.controller.load();

    mounted.installButton.click();
    await vi.waitFor(() => expect(mounted.notify).toHaveBeenCalledWith("写入扩展目录失败"));
    expect(getStatus).toHaveBeenCalledTimes(2);
    expect(mounted.installButton.disabled).toBe(false);
  });

  it("shows the backend error in the card when the status query itself fails", async () => {
    const mounted = mount({ getStatus: vi.fn().mockRejectedValue("D-Bus 不可用") });

    await mounted.controller.load();
    expect(mounted.detailText.textContent).toBe("D-Bus 不可用");
    expect(mounted.installButton.hidden).toBe(false);
  });

  it("re-renders the last status when the language changes", async () => {
    const getStatus = vi.fn().mockResolvedValue(status({ active: true }));
    const mounted = mount({ getStatus });

    await mounted.controller.load();
    mounted.stateText.textContent = "stale";
    mounted.controller.refreshLabels();
    expect(mounted.stateText.textContent).toBe("settings.windowProbe.stateActive");
    // 只重画，不再问一次后端
    expect(getStatus).toHaveBeenCalledTimes(1);
  });
});

describe("capture diagnostics card", () => {
  function mount({ collect, copyText, openUrl } = {}) {
    const noteInput = document.createElement("input");
    const runButton = document.createElement("button");
    const copyButton = document.createElement("button");
    const reportButton = document.createElement("button");
    const pathText = document.createElement("div");
    const output = document.createElement("pre");
    runButton.textContent = "Run Diagnostics";
    copyButton.hidden = true;
    reportButton.hidden = true;
    output.hidden = true;
    pathText.hidden = true;
    document.body.replaceChildren(
      noteInput,
      runButton,
      copyButton,
      reportButton,
      pathText,
      output,
    );
    const notify = vi.fn();
    const controller = createCaptureDiagnosticsCard({
      noteInput,
      runButton,
      copyButton,
      reportButton,
      pathText,
      output,
      collect: collect ?? vi.fn(),
      copyText: copyText ?? vi.fn().mockResolvedValue(undefined),
      openUrl: openUrl ?? vi.fn().mockResolvedValue(undefined),
      translate,
      notify,
    });
    return {
      controller,
      noteInput,
      runButton,
      copyButton,
      reportButton,
      pathText,
      output,
      notify,
    };
  }

  const report = {
    text: "=== Clippy capture geometry ===\nI4   overlay viewport   FAIL",
    path: "/home/u/.cache/clippy/capture-diagnostics.txt",
    fixtureJson: '{"name":"monitor-layout"}',
  };

  // 打开设置页不能顺手跑一遍诊断：里面有一次真实的舞台图请求（约半秒）。
  it("collects nothing until the user asks for it", () => {
    const collect = vi.fn().mockResolvedValue(report);
    const mounted = mount({ collect });
    expect(collect).not.toHaveBeenCalled();
    expect(mounted.output.hidden).toBe(true);
    expect(mounted.copyButton.hidden).toBe(true);
  });

  it("passes the note along and shows the report plus where it landed", async () => {
    const collect = vi.fn().mockResolvedValue(report);
    const mounted = mount({ collect });
    mounted.noteInput.value = "  laptop offset up-left  ";

    mounted.runButton.click();
    await vi.waitFor(() => expect(mounted.output.hidden).toBe(false));
    expect(collect).toHaveBeenCalledWith("laptop offset up-left");
    expect(mounted.output.textContent).toBe(report.text);
    expect(mounted.pathText.hidden).toBe(false);
    expect(mounted.pathText.textContent).toContain(report.path);
    expect(mounted.copyButton.hidden).toBe(false);
    expect(mounted.reportButton.hidden).toBe(false);
    // 按钮恢复原文案，不能一直停在"采集中"
    expect(mounted.runButton.disabled).toBe(false);
    expect(mounted.runButton.textContent).toBe("Run Diagnostics");
  });

  it("sends null instead of an empty note", async () => {
    const collect = vi.fn().mockResolvedValue(report);
    const mounted = mount({ collect });
    mounted.noteInput.value = "   ";

    mounted.runButton.click();
    await vi.waitFor(() => expect(collect).toHaveBeenCalledWith(null));
  });

  // 报告是后端来的纯文本，只能走 textContent——里面带用户环境信息，绝不进 innerHTML。
  it("writes the report as text, never as markup", async () => {
    const collect = vi.fn().mockResolvedValue({
      ...report,
      text: "<img src=x onerror=alert(1)>",
    });
    const mounted = mount({ collect });

    mounted.runButton.click();
    await vi.waitFor(() => expect(mounted.output.hidden).toBe(false));
    expect(mounted.output.querySelector("img")).toBeNull();
    expect(mounted.output.textContent).toBe("<img src=x onerror=alert(1)>");
  });

  it("shows the failure instead of a blank box when collection itself fails", async () => {
    const mounted = mount({ collect: vi.fn().mockRejectedValue("拿不到显示器几何") });

    mounted.runButton.click();
    await vi.waitFor(() => expect(mounted.output.textContent).toBe("拿不到显示器几何"));
    // 没有报告就没有可复制、可提交的东西
    expect(mounted.copyButton.hidden).toBe(true);
    expect(mounted.reportButton.hidden).toBe(true);
    expect(mounted.runButton.disabled).toBe(false);
  });

  // 报告有两三千字符，塞进 URL 会被静默截断——截断的诊断报告比没有报告更糟。
  it("copies the report and opens the plain issue template without stuffing it into the URL", async () => {
    const copyText = vi.fn().mockResolvedValue(undefined);
    const openUrl = vi.fn().mockResolvedValue(undefined);
    const mounted = mount({ collect: vi.fn().mockResolvedValue(report), copyText, openUrl });
    await mounted.controller.run();

    mounted.reportButton.click();
    await vi.waitFor(() => expect(openUrl).toHaveBeenCalledWith(CAPTURE_ISSUE_URL));
    expect(copyText).toHaveBeenCalledWith(report.text);
    expect(CAPTURE_ISSUE_URL).not.toContain(encodeURIComponent("I4"));
    expect(CAPTURE_ISSUE_URL).toContain("template=capture-geometry.yml");
  });

  it("does nothing on copy or report before a report exists", async () => {
    const copyText = vi.fn();
    const openUrl = vi.fn();
    const mounted = mount({ copyText, openUrl });

    mounted.copyButton.click();
    mounted.reportButton.click();
    await Promise.resolve();
    expect(copyText).not.toHaveBeenCalled();
    expect(openUrl).not.toHaveBeenCalled();
  });

  it("keeps quiet about a file that was never written", () => {
    expect(describeReportPath({ ...report, path: null }, translate)).toBeNull();
    expect(describeReportPath(null, translate)).toBeNull();
    expect(describeReportPath(report, translate)).toContain(report.path);
  });
});

describe("settings tabs", () => {
  function mount() {
    const root = document.createElement("div");
    root.innerHTML = `
      <div class="settings-tabs" role="tablist" id="settings-tabs">
        <button class="settings-tab" role="tab" data-settings-tab="general">General</button>
        <button class="settings-tab" role="tab" data-settings-tab="screenshot">Screenshot</button>
        <button class="settings-tab" role="tab" data-settings-tab="about">About</button>
      </div>
      <div class="settings-panel" data-settings-panel="general"></div>
      <div class="settings-panel" data-settings-panel="screenshot"></div>
      <div class="settings-panel" data-settings-panel="about"></div>
    `;
    document.body.replaceChildren(root);
    const tabs = [...root.querySelectorAll("[data-settings-tab]")];
    const panels = [...root.querySelectorAll("[data-settings-panel]")];
    return { root, tabs, panels, controller: initSettingsTabs(root) };
  }

  beforeEach(() => {
    window.localStorage.clear();
  });

  it("shows the first page and only that page", () => {
    const { tabs, panels } = mount();

    expect(panels.map((panel) => panel.hidden)).toEqual([false, true, true]);
    expect(tabs.map((tab) => tab.getAttribute("aria-selected"))).toEqual(["true", "false", "false"]);
    // roving tabindex：只有当前分页在 Tab 序列里
    expect(tabs.map((tab) => tab.tabIndex)).toEqual([0, -1, -1]);
  });

  it("switches pages on click without destroying the hidden panels", () => {
    const { tabs, panels } = mount();

    tabs[1].click();
    expect(panels.map((panel) => panel.hidden)).toEqual([true, false, true]);
    // 各控制器在装配时就抓住了面板里的元素，面板一旦被移除引用就失效了
    expect(panels[0].isConnected).toBe(true);
  });

  it("moves between pages with the arrow keys and wraps around", () => {
    const { tabs, panels } = mount();

    tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(panels[1].hidden).toBe(false);
    tabs[1].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    expect(panels[0].hidden).toBe(false);
    tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    expect(panels[2].hidden).toBe(false);
    tabs[2].dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    expect(panels[0].hidden).toBe(false);
    tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    expect(panels[2].hidden).toBe(false);
  });

  it("comes back to the page the user left on, and ignores a stale one", () => {
    mount().tabs[1].click();
    expect(mount().panels[1].hidden).toBe(false);

    window.localStorage.setItem("clippy.settings.tab", "translation");
    // 分页改名后旧记录会指向不存在的面板，此时退回第一页而不是全部隐藏
    expect(mount().panels.map((panel) => panel.hidden)).toEqual([false, true, true]);
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
