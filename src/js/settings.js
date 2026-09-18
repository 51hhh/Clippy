/** settings.js - 设置页装配与配置保存。 */

import {
  checkShortcutConflict,
  closeSettings,
  copyText,
  disableAutostart,
  enableAutostart,
  getAppVersion,
  getConfig,
  getOcrHealthStatus,
  getPasteStatus,
  getPlatformInfo,
  getShortcutFailures,
  getStats,
  getWindowProbeStatus,
  installWindowProbeExtension,
  isAutostartEnabled,
  isDevBinary,
  ocrInstall,
  onShortcutRegisterFailed,
  openExternalUrl,
  pauseShortcuts,
  pickScreenshotDirectory,
  pickOcrManifest,
  requestPastePermission,
  resumeShortcuts,
  runCaptureDiagnostics,
  tmuxAvailable,
  toggleTmuxCapture,
  uninstallWindowProbeExtension,
  updateConfig,
} from "./api.ts";
import { initCustomSelect } from "./custom-select.js";
import { createAutostartSettings } from "./settings/autostart-settings.js";
import { createCaptureDiagnosticsCard } from "./settings/capture-diagnostics.js";
import { createOcrSettings } from "./settings/ocr-settings.js";
import { createPastePermissionController } from "./settings/paste-permission.js";
import { createPlatformCapabilities } from "./settings/platform-capabilities.js";
import { createScreenshotSettings } from "./settings/screenshot-settings.js";
import { createShortcutFailureNotice } from "./settings/shortcut-failure-notice.js";
import {
  closeAfterShortcutCleanup,
  saveAfterShortcutCleanup,
  shortcutSaveErrorMessage,
  createShortcutRecordingController,
} from "./settings/shortcut-recording.js";
import { loadStats } from "./settings/stats.js";
import { initSettingsTabs } from "./settings/tabs.js";
import { createConfigWriter } from "./settings/config-writer.js";
import { createThemePicker } from "./settings/theme-picker.js";
import { createWindowProbeCard } from "./settings/window-probe.js";
import { initTranslationSettings } from "./translation-settings.js";
import { checkForUpdate, initUpdateModal } from "./update-modal.js";
import * as i18n from "../i18n/i18n.js";
import "../styles/themes.css";
import "../styles/base.css";
import "../styles/settings.css";

function element(id) {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Missing settings element: #${id}`);
  return found;
}

const shortcutInput = element("shortcut-input");
const pinShortcutInput = element("pin-shortcut-input");
const captureShortcutInput = element("capture-shortcut-input");
const maxHistoryInput = element("max-history-input");
const languageSelect = element("language-select");
const autostartToggle = element("autostart-toggle");
const autoPasteToggle = element("auto-paste-toggle");
const tmuxGroup = element("tmux-group");
const tmuxToggle = element("tmux-toggle");
const toast = element("toast");
const ocrModeControl = initCustomSelect(element("ocr-mode-select"));

let savedConfig = null;
let operatingSystem = null;

function showToast(message) {
  toast.textContent = message;
  toast.classList.remove("hidden");
  void toast.offsetWidth;
  toast.classList.add("show");
  setTimeout(() => {
    toast.classList.remove("show");
    setTimeout(() => toast.classList.add("hidden"), 300);
  }, 2000);
}

const translationSettings = initTranslationSettings({ showToast });

const configWriter = createConfigWriter({ getConfig, updateConfig, onSaved: config => { savedConfig = config; } });
const themePicker = createThemePicker({
  container: element("theme-grid"),
  translate: i18n.t,
  persistTheme: theme => configWriter.write({ theme }),
  notify: showToast,
});

const pastePermission = createPastePermissionController({
  statusDot: element("paste-status-dot"),
  statusText: element("paste-status-text"),
  authorizeButton: element("paste-authorize-btn"),
  getStatus: getPasteStatus,
  requestPermission: requestPastePermission,
  translate: i18n.t,
});

const shortcutRecording = createShortcutRecordingController({
  notify: () => showToast(i18n.t("settings.shortcut.restoreFailed")),
  pauseShortcuts,
  resumeShortcuts,
  translate: i18n.t,
  metaModifier: () => (operatingSystem === "macos" ? "Command" : "Super"),
  recorders: {
    global: {
      input: shortcutInput,
      recordButton: element("shortcut-record-btn"),
      clearButton: element("shortcut-clear-btn"),
      warning: element("shortcut-warning"),
      defaultValue: "",
      getSavedValue: () => savedConfig?.global_shortcut || "",
      checkConflict: checkShortcutConflict,
    },
    pin: {
      input: pinShortcutInput,
      recordButton: element("pin-shortcut-record-btn"),
      clearButton: element("pin-shortcut-clear-btn"),
      warning: element("pin-shortcut-warning"),
      defaultValue: "Ctrl+2",
      getSavedValue: () => savedConfig?.pin_shortcut || "Ctrl+2",
      checkConflict: checkShortcutConflict,
    },
    capture: {
      input: captureShortcutInput,
      recordButton: element("capture-shortcut-record-btn"),
      clearButton: element("capture-shortcut-clear-btn"),
      warning: element("capture-shortcut-warning"),
      defaultValue: "Ctrl+Shift+S",
      getSavedValue: () => savedConfig?.capture_shortcut || "Ctrl+Shift+S",
      checkConflict: checkShortcutConflict,
    },
  },
});

const shortcutFailureNotice = createShortcutFailureNotice({
  warning: element("shortcut-register-warning"),
  translate: i18n.t,
});
void onShortcutRegisterFailed((failure) => shortcutFailureNotice.add(failure));

const ocrSettings = createOcrSettings({
  toggle: element("ocr-toggle"),
  card: element("ocr-status-card"),
  statusDot: element("ocr-status-dot"),
  statusText: element("ocr-status-text"),
  detailText: element("ocr-status-detail"),
  engineText: element("ocr-engine-text"),
  pipelineRow: element("ocr-pipeline-row"),
  pipelineText: element("ocr-pipeline-text"),
  fallbackText: element("ocr-fallback-text"),
  manifestInput: element("ocr-manifest-input"),
  browseButton: element("ocr-manifest-browse-btn"),
  clearButton: element("ocr-manifest-clear-btn"),
  recheckButton: element("ocr-recheck-btn"),
  installButton: element("ocr-install-btn"),
  options: element("ocr-options"),
  modeControl: ocrModeControl,
  getStatus: getOcrHealthStatus,
  pickManifest: pickOcrManifest,
  install: ocrInstall,
  translate: i18n.t,
  showToast,
});

const windowProbe = createWindowProbeCard({
  card: element("window-probe-card"),
  dot: element("window-probe-dot"),
  stateText: element("window-probe-state"),
  detailText: element("window-probe-detail"),
  installButton: element("window-probe-install-btn"),
  uninstallButton: element("window-probe-uninstall-btn"),
  recheckButton: element("window-probe-recheck-btn"),
  getStatus: getWindowProbeStatus,
  install: installWindowProbeExtension,
  uninstall: uninstallWindowProbeExtension,
  translate: i18n.t,
  notify: showToast,
});

const platformCapabilities = createPlatformCapabilities({
  summary: element("platform-summary"),
  portal: element("platform-portal-summary"),
  list: element("platform-capability-list"),
  translate: i18n.t,
});

async function loadPlatformCapabilities() {
  try {
    const platform = await getPlatformInfo();
    operatingSystem = platform.operating_system;
    platformCapabilities.render(platform);
    ocrSettings.setPlatform(platform.operating_system);
    await ocrSettings.checkStatus();
  } catch (error) {
    console.warn("读取平台能力失败:", error);
    platformCapabilities.renderError();
    // 平台未知时不暴露安装按钮，避免在错误系统上调用 Linux 包管理器。
  }
}

// 只有用户点"Run Diagnostics"才会采集：里面有一次真实的舞台图请求，打开设置页不该付这个钱。
createCaptureDiagnosticsCard({
  noteInput: element("capture-diagnostics-note"),
  runButton: element("capture-diagnostics-run-btn"),
  copyButton: element("capture-diagnostics-copy-btn"),
  reportButton: element("capture-diagnostics-report-btn"),
  pathText: element("capture-diagnostics-path"),
  output: element("capture-diagnostics-output"),
  collect: runCaptureDiagnostics,
  copyText,
  openUrl: openExternalUrl,
  translate: i18n.t,
  notify: showToast,
});

const screenshotSettings = createScreenshotSettings({
  directoryInput: element("screenshot-dir-input"),
  browseButton: element("screenshot-dir-browse-btn"),
  templateInput: element("screenshot-template-input"),
  pickDirectory: pickScreenshotDirectory,
  translate: i18n.t,
  showToast,
});

function fillForm(config) {
  shortcutRecording.setValues({
    global: config.global_shortcut || "",
    pin: config.pin_shortcut || "Ctrl+2",
    capture: config.capture_shortcut || "Ctrl+Shift+S",
  });
  maxHistoryInput.value = config.max_history ?? 100;
  languageSelect.value = config.language || "auto";
  ocrSettings.fill(config);
  screenshotSettings.fill(config);
  tmuxToggle.checked = config.tmux_capture === true;
  autoPasteToggle.checked = config.auto_paste !== false;
  translationSettings.fill(config);
}

const autostartSettings = createAutostartSettings({
  toggle: autostartToggle,
  isDevBinary,
  isEnabled: isAutostartEnabled,
  enable: enableAutostart,
  disable: disableAutostart,
  translate: i18n.t,
  notify: showToast,
});

/** 主动拉取存量注册失败记录：启动阶段的失败早于本页监听，事件已经丢了 */
async function refreshShortcutFailures() {
  try {
    shortcutFailureNotice.replaceAll(await getShortcutFailures());
  } catch (error) {
    console.warn("读取快捷键注册状态失败:", error);
  }
}

function whenReady(callback) {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", callback);
  } else {
    callback();
  }
}

whenReady(async () => {
  initSettingsTabs();
  try {
    savedConfig = await getConfig();
    fillForm(savedConfig);
    themePicker.initialize(savedConfig.theme || "light");
    i18n.init(savedConfig.language || "auto");
    translationSettings.refreshLabels();
    await Promise.all([
      pastePermission.load(),
      windowProbe.load(),
      translationSettings.loadKeyStatus(),
      loadPlatformCapabilities(),
    ]);

    try {
      element("about-version").textContent = `v${await getAppVersion()}`;
    } catch (error) {
      console.warn("获取版本号失败:", error);
    }
    await autostartSettings.load();
    void refreshShortcutFailures();
    void loadStats({
      getStats,
      elements: {
        total: element("stats-total"),
        favorites: element("stats-favorites"),
        text: element("stats-text"),
        html: element("stats-html"),
        image: element("stats-image"),
        size: element("stats-size"),
      },
    });
  } catch (error) {
    console.error("加载配置失败:", error);
    themePicker.initialize("light");
    i18n.init("auto");
    translationSettings.refreshLabels();
    void windowProbe.load();
    void translationSettings.loadKeyStatus();
  }
});

languageSelect.addEventListener("change", () => {
  i18n.init(languageSelect.value);
  themePicker.refreshLabels();
  shortcutRecording.refreshLabels();
  shortcutFailureNotice.refreshLabels();
  pastePermission.refreshLabels();
  windowProbe.refreshLabels();
  ocrSettings.refreshLabels();
  translationSettings.refreshLabels();
});

element("save-btn").addEventListener("click", async (event) => {
  const button = event.currentTarget;
  if (button.disabled) return;
  button.disabled = true;
  try {
    const outcome = await saveAfterShortcutCleanup(shortcutRecording, () => {
      const shortcuts = shortcutRecording.getValues();
      return configWriter.write({
        max_history: parseInt(maxHistoryInput.value, 10) || 0,
        global_shortcut: shortcuts.global,
        pin_shortcut: shortcuts.pin,
        capture_shortcut: shortcuts.capture,
        language: languageSelect.value,
        ...ocrSettings.getConfig(),
        ...screenshotSettings.getConfig(),
        auto_paste: autoPasteToggle.checked,
        ...translationSettings.getConfig(),
      }, { requireShortcutsActive: true });
    });
    await refreshShortcutFailures();
    showToast(i18n.t(outcome?.shortcut_status === "pending" ? "settings.savedPending" : "settings.saved"));
  } catch (error) {
    console.error("保存失败:", error);
    await refreshShortcutFailures();
    showToast(shortcutSaveErrorMessage(error, i18n.t));
  } finally {
    button.disabled = false;
  }
});

initUpdateModal();
element("check-update-btn").addEventListener("click", async (event) => {
  const button = event.currentTarget;
  const status = element("update-status");
  button.disabled = true;
  status.classList.add("hidden");
  status.classList.remove("error");
  try {
    if (!(await checkForUpdate(true))) {
      status.textContent = i18n.t("settings.about.upToDate");
      status.classList.remove("hidden");
      setTimeout(() => status.classList.add("hidden"), 3000);
    }
  } catch (error) {
    console.warn("检查更新失败:", error);
    status.textContent = i18n.t("settings.about.checkFailed");
    status.classList.remove("hidden");
    status.classList.add("error");
    setTimeout(() => status.classList.add("hidden"), 3000);
  } finally {
    button.disabled = false;
  }
});

let closingSettings = false;
async function requestSettingsClose() {
  if (closingSettings) return;
  closingSettings = true;
  try {
    await closeAfterShortcutCleanup(shortcutRecording, closeSettings);
  } catch (error) {
    console.warn(error);
    showToast(i18n.t("settings.shortcut.restoreFailed"));
  } finally {
    closingSettings = false;
  }
}
element("cancel-btn").addEventListener("click", () => { void requestSettingsClose(); });

tmuxToggle.addEventListener("change", async () => {
  try {
    const enabled = tmuxToggle.checked;
    tmuxToggle.disabled = true;
    await configWriter.run(async () => {
      await toggleTmuxCapture(enabled);
      savedConfig = await getConfig();
    });
  } catch (error) {
    console.warn("tmux 切换失败:", error);
    tmuxToggle.checked = !tmuxToggle.checked;
    showToast(String(error?.message || error || "tmux error"));
  } finally {
    tmuxToggle.disabled = false;
  }
});

void tmuxAvailable()
  .then((available) => {
    if (available) tmuxGroup.hidden = false;
  })
  .catch(() => {
    // tmux 不可用时保持面板隐藏。
  });
