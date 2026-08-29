/** settings.js - 设置页装配与配置保存。 */

import {
  checkShortcutConflict,
  closeCurrentWindow,
  disableAutostart,
  enableAutostart,
  getAppVersion,
  getConfig,
  getPasteStatus,
  getShortcutFailures,
  getStats,
  isAutostartEnabled,
  isDevBinary,
  ocrAvailable,
  ocrInstall,
  onShortcutRegisterFailed,
  pauseShortcuts,
  pickScreenshotDirectory,
  requestPastePermission,
  resumeShortcuts,
  tmuxAvailable,
  toggleTmuxCapture,
  updateConfig,
} from "./api.ts";
import { initCustomSelect } from "./custom-select.js";
import { createOcrSettings } from "./settings/ocr-settings.js";
import { createPastePermissionController } from "./settings/paste-permission.js";
import { createScreenshotSettings } from "./settings/screenshot-settings.js";
import { createShortcutFailureNotice } from "./settings/shortcut-failure-notice.js";
import {
  closeAfterShortcutCleanup,
  createShortcutRecordingController,
} from "./settings/shortcut-recording.js";
import { loadStats } from "./settings/stats.js";
import { createThemePicker } from "./settings/theme-picker.js";
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

const themePicker = createThemePicker({
  container: element("theme-grid"),
  translate: i18n.t,
  async persistTheme(theme) {
    if (!savedConfig) return;
    const nextConfig = { ...savedConfig, theme };
    await updateConfig(nextConfig);
    savedConfig = nextConfig;
  },
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
  pauseShortcuts,
  resumeShortcuts,
  translate: i18n.t,
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
  statusDot: element("ocr-status-dot"),
  statusText: element("ocr-status-text"),
  installButton: element("ocr-install-btn"),
  options: element("ocr-options"),
  modeControl: ocrModeControl,
  checkAvailable: ocrAvailable,
  install: ocrInstall,
  translate: i18n.t,
  showToast,
});

const screenshotSettings = createScreenshotSettings({
  directoryInput: element("screenshot-dir-input"),
  browseButton: element("screenshot-dir-browse-btn"),
  templateInput: element("screenshot-template-input"),
  commitActionControl: initCustomSelect(element("capture-commit-action-select")),
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

async function loadAutostartStatus() {
  try {
    if (await isDevBinary()) {
      autostartToggle.checked = false;
      const hint = autostartToggle
        .closest(".setting-toggle-row")
        ?.querySelector(".setting-hint");
      if (hint) {
        hint.textContent = `${hint.textContent} ${i18n.t("settings.autostart.devHint")}`;
      }
      try {
        await disableAutostart();
      } catch (error) {
        console.warn("清理 dev 自启项失败:", error);
      }
    } else {
      autostartToggle.checked = await isAutostartEnabled();
    }
  } catch (error) {
    console.warn("获取自启动状态失败:", error);
  }
}

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
  try {
    savedConfig = await getConfig();
    fillForm(savedConfig);
    themePicker.initialize(savedConfig.theme || "light");
    i18n.init(savedConfig.language || "auto");
    translationSettings.refreshLabels();
    await Promise.all([pastePermission.load(), translationSettings.loadKeyStatus()]);

    try {
      element("about-version").textContent = `v${await getAppVersion()}`;
    } catch (error) {
      console.warn("获取版本号失败:", error);
    }
    await loadAutostartStatus();
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
    void translationSettings.loadKeyStatus();
  }
});

languageSelect.addEventListener("change", () => {
  i18n.init(languageSelect.value);
  themePicker.refreshLabels();
  shortcutRecording.refreshLabels();
  shortcutFailureNotice.refreshLabels();
  pastePermission.refreshLabels();
  translationSettings.refreshLabels();
});

autostartToggle.addEventListener("change", async () => {
  try {
    if (autostartToggle.checked) {
      await enableAutostart();
    } else {
      await disableAutostart();
    }
  } catch (error) {
    console.warn("切换自启动失败:", error);
    autostartToggle.checked = !autostartToggle.checked;
  }
});

element("save-btn").addEventListener("click", async () => {
  const shortcuts = shortcutRecording.getValues();
  try {
    const newConfig = {
      // 这里写出的始终是 v2 结构（translation_services 列表），版本号不能回退到 1。
      version: savedConfig?.version ?? 2,
      max_history: parseInt(maxHistoryInput.value, 10) || 0,
      storage_mode: savedConfig?.storage_mode || "persistent",
      global_shortcut: shortcuts.global || savedConfig?.global_shortcut || "Super+V",
      pin_shortcut: shortcuts.pin || savedConfig?.pin_shortcut || "Ctrl+2",
      capture_shortcut:
        shortcuts.capture || savedConfig?.capture_shortcut || "Ctrl+Shift+S",
      theme: themePicker.value,
      language: languageSelect.value,
      delete_confirm_ms: savedConfig?.delete_confirm_ms ?? 1200,
      ...ocrSettings.getConfig(),
      ...screenshotSettings.getConfig(),
      tmux_capture: tmuxGroup.hidden
        ? (savedConfig?.tmux_capture ?? false)
        : tmuxToggle.checked,
      auto_paste: autoPasteToggle.checked,
      ...translationSettings.getConfig(),
      main_window_position: savedConfig?.main_window_position ?? null,
    };

    await updateConfig(newConfig);
    savedConfig = newConfig;
    // 后端在 update_config 里同步记账，返回后查到的就是这次保存的真实结果。
    await refreshShortcutFailures();
    showToast(i18n.t("settings.saved"));
  } catch (error) {
    console.error("保存失败:", error);
    showToast(i18n.t("settings.saveFailed", { error }));
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

element("cancel-btn").addEventListener("click", async () => {
  try {
    await closeAfterShortcutCleanup(shortcutRecording, closeCurrentWindow);
  } catch (error) {
    console.warn(error);
  }
});

tmuxToggle.addEventListener("change", async () => {
  try {
    await toggleTmuxCapture(tmuxToggle.checked);
  } catch (error) {
    console.warn("tmux 切换失败:", error);
    tmuxToggle.checked = !tmuxToggle.checked;
    showToast(String(error?.message || error || "tmux error"));
  }
});

void ocrSettings.checkStatus();
void tmuxAvailable()
  .then((available) => {
    if (available) tmuxGroup.hidden = false;
  })
  .catch(() => {
    // tmux 不可用时保持面板隐藏。
  });
