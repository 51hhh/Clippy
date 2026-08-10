/**
 * settings.js — 设置面板逻辑
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  enable as enableAutostart,
  disable as disableAutostart,
  isEnabled as isAutostartEnabled,
} from "@tauri-apps/plugin-autostart";
import {
  checkShortcutConflict,
  getConfig,
  getAppVersion,
  getPasteStatus,
  getStats,
  isDevBinary,
  ocrAvailable,
  ocrInstall,
  pauseShortcuts,
  requestPastePermission,
  resumeShortcuts,
  toggleTmuxCapture,
  tmuxAvailable,
  updateConfig,
} from "./api.js";
import { keyEventToShortcut } from "./shortcut-recorder.js";
import { initCustomSelect } from "./custom-select.js";
import { initUpdateModal, checkForUpdate } from "./update-modal.js";
import { initTranslationSettings } from "./translation-settings.js";
import * as i18n from "../i18n/i18n.js";
import "../styles/themes.css";
import "../styles/base.css";
import "../styles/settings.css";

// DOM 引用
const shortcutInput   = document.getElementById("shortcut-input");
const recordBtn       = document.getElementById("shortcut-record-btn");
const clearBtn        = document.getElementById("shortcut-clear-btn");
const shortcutWarning = document.getElementById("shortcut-warning");
const pinShortcutInput = document.getElementById("pin-shortcut-input");
const pinRecordBtn     = document.getElementById("pin-shortcut-record-btn");
const pinClearBtn      = document.getElementById("pin-shortcut-clear-btn");
const captureShortcutInput = document.getElementById("capture-shortcut-input");
const captureRecordBtn     = document.getElementById("capture-shortcut-record-btn");
const captureClearBtn      = document.getElementById("capture-shortcut-clear-btn");
const themeGrid       = document.getElementById("theme-grid");
const maxHistoryInput = document.getElementById("max-history-input");
const languageSelect  = document.getElementById("language-select");
const saveBtn         = document.getElementById("save-btn");
const cancelBtn       = document.getElementById("cancel-btn");
const toast           = document.getElementById("toast");
const aboutVersion    = document.getElementById("about-version");
const checkUpdateBtn  = document.getElementById("check-update-btn");
const updateStatus    = document.getElementById("update-status");
const autostartToggle = document.getElementById("autostart-toggle");
const autoPasteToggle = document.getElementById("auto-paste-toggle");
const pasteStatusDot  = document.getElementById("paste-status-dot");
const pasteStatusText = document.getElementById("paste-status-text");
const pasteAuthorizeBtn = document.getElementById("paste-authorize-btn");
const ocrModeSelect  = document.getElementById("ocr-mode-select");
const ocrToggle      = document.getElementById("ocr-toggle");
const ocrStatusDot   = document.getElementById("ocr-status-dot");
const ocrStatusText  = document.getElementById("ocr-status-text");
const ocrInstallBtn  = document.getElementById("ocr-install-btn");
const ocrOptions     = document.getElementById("ocr-options");
const tmuxGroup      = document.getElementById("tmux-group");
const tmuxToggle     = document.getElementById("tmux-toggle");

// ── 自定义下拉框 ──
const ocrModeCtrl = initCustomSelect(ocrModeSelect);

// 主题清单：id 与 themes.css 中 [data-theme="<id>"] 对应；i18nKey 用于显示名
const THEMES = [
  { id: "light",            i18nKey: "settings.theme.light" },
  { id: "dark",             i18nKey: "settings.theme.dark" },
  { id: "nord",             i18nKey: "settings.theme.nord" },
  { id: "solarized-light",  i18nKey: "settings.theme.solarizedLight" },
  { id: "rose",             i18nKey: "settings.theme.rose" },
  { id: "midnight",         i18nKey: "settings.theme.midnight" },
];
let selectedTheme = "light";

let savedConfig = null;
let isRecording = false;
let isPinRecording = false;
let isCaptureRecording = false;
let lastPasteStatus = null;

const translationSettings = initTranslationSettings({ showToast });

// 初始化
function whenReady(fn) {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", fn);
  } else {
    fn();
  }
}

whenReady(async () => {
  try {
    savedConfig = await getConfig();
    fillForm(savedConfig);
    selectedTheme = savedConfig.theme || "light";
    renderThemeGrid();
    applyTheme(selectedTheme);
    i18n.init(savedConfig.language || "auto");
    translationSettings.refreshLabels();
    await Promise.all([loadPasteStatus(), translationSettings.loadKeyStatus()]);
    // 显示版本号
    try {
      const ver = await getAppVersion();
      if (aboutVersion) aboutVersion.textContent = `v${ver}`;
    } catch (e) { console.warn("获取版本号失败:", e); }
    // 加载开机自启动状态 —— dev 二进制禁止开启自启（避免污染 ~/.config/autostart/Clippy.desktop）
    try {
      const dev = await isDevBinary();
      if (dev) {
        autostartToggle.checked = false;
        // dev 模式下显示提示，但不禁用开关（安装版不受影响）
        const hint = autostartToggle.closest(".setting-toggle-row")?.querySelector(".setting-hint");
        if (hint) hint.textContent = hint.textContent + " " + i18n.t("settings.autostart.devHint");
        // 顺手清理已被错误写入的 dev 路径自启项
        try { await disableAutostart(); } catch (e) { console.warn("清理 dev 自启项失败:", e); }
      } else {
        autostartToggle.checked = await isAutostartEnabled();
      }
    } catch (e) { console.warn("获取自启动状态失败:", e); }
    // 加载统计数据
    loadStats();
  } catch (err) {
    console.error("加载配置失败:", err);
    selectedTheme = "light";
    renderThemeGrid();
    i18n.init("auto");
    translationSettings.refreshLabels();
    translationSettings.loadKeyStatus();
  }
});

function fillForm(config) {
  shortcutInput.value    = config.global_shortcut || "";
  pinShortcutInput.value = config.pin_shortcut || "Ctrl+2";
  captureShortcutInput.value = config.capture_shortcut || "Ctrl+Shift+S";
  maxHistoryInput.value  = config.max_history ?? 100;
  languageSelect.value   = config.language || "auto";
  ocrModeCtrl.value    = config.ocr_result_mode || "preview";
  ocrToggle.checked      = config.ocr_enabled !== false;
  tmuxToggle.checked     = config.tmux_capture === true;
  autoPasteToggle.checked = config.auto_paste !== false;
  translationSettings.fill(config);
  updateOcrOptionsVisibility();
}

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
}

/** 实时把主题持久化并广播，让主窗口同步刷新（无需点 Save）。 */
async function persistTheme(theme) {
  selectedTheme = theme;
  applyTheme(theme);
  if (!savedConfig) return;
  const next = { ...savedConfig, theme };
  try {
    await updateConfig(next);
    savedConfig = next;
  } catch (err) {
    console.warn("主题持久化失败:", err);
  }
}

function renderThemeGrid() {
  if (!themeGrid) return;
  themeGrid.replaceChildren();
  for (const theme of THEMES) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = "theme-card";
    card.dataset.theme = theme.id;
    card.setAttribute("role", "radio");
    card.setAttribute("aria-checked", String(theme.id === selectedTheme));
    if (theme.id === selectedTheme) card.classList.add("selected");

    // 用嵌套 div 充当真实预览：内层应用 data-theme，复用 themes.css 变量
    const preview = document.createElement("div");
    preview.className = "theme-preview";
    preview.dataset.theme = theme.id;
    preview.innerHTML = `
      <div class="tp-bar"></div>
      <div class="tp-row"><span class="tp-dot"></span><span class="tp-line"></span></div>
      <div class="tp-row tp-row-active"><span class="tp-dot tp-dot-accent"></span><span class="tp-line tp-line-strong"></span></div>
      <div class="tp-row"><span class="tp-dot"></span><span class="tp-line tp-line-short"></span></div>
    `;

    const label = document.createElement("span");
    label.className = "theme-name";
    label.dataset.i18n = theme.i18nKey;
    label.textContent = i18n.t(theme.i18nKey);

    card.append(preview, label);
    card.addEventListener("click", () => selectTheme(theme.id));
    themeGrid.appendChild(card);
  }
}

function selectTheme(theme) {
  for (const card of themeGrid.querySelectorAll(".theme-card")) {
    const isSel = card.dataset.theme === theme;
    card.classList.toggle("selected", isSel);
    card.setAttribute("aria-checked", String(isSel));
  }
  persistTheme(theme).catch(console.warn);
}

// 语言预览
languageSelect.addEventListener("change", () => {
  i18n.init(languageSelect.value);
  renderThemeGrid();
  if (isRecording) {
    recordBtn.textContent = i18n.t("settings.shortcut.stop");
  }
  if (isPinRecording) {
    pinRecordBtn.textContent = i18n.t("settings.shortcut.stop");
  }
  if (isCaptureRecording) {
    captureRecordBtn.textContent = i18n.t("settings.shortcut.stop");
  }
  if (lastPasteStatus) renderPasteStatus(lastPasteStatus);
  translationSettings.refreshLabels();
});

// 开机自启动 toggle — 立即生效，无需点 Save
autostartToggle.addEventListener("change", async () => {
  try {
    if (autostartToggle.checked) {
      await enableAutostart();
    } else {
      await disableAutostart();
    }
  } catch (e) {
    console.warn("切换自启动失败:", e);
    autostartToggle.checked = !autostartToggle.checked;
  }
});

async function loadPasteStatus() {
  try {
    renderPasteStatus(await getPasteStatus());
  } catch (error) {
    renderPasteStatus({ backend: "copy_only", phase: "unavailable", detail: String(error) });
  }
}

function renderPasteStatus(status) {
  lastPasteStatus = status;
  const backend = status?.backend || "copy_only";
  const phase = status?.phase || "unavailable";
  let key = "settings.autoPaste.unavailable";
  let tone = "unavailable";

  if (backend === "x11") {
    key = "settings.autoPaste.x11Ready";
    tone = "ready";
  } else if (backend === "wayland_portal" && phase === "ready") {
    key = "settings.autoPaste.portalReady";
    tone = "ready";
  } else if (backend === "wayland_portal" && phase === "initializing") {
    key = "settings.autoPaste.initializing";
    tone = "pending";
  } else if (backend === "wayland_portal" && phase === "denied") {
    key = "settings.autoPaste.denied";
  } else if (backend === "wayland_portal") {
    key = "settings.autoPaste.permissionRequired";
    tone = "pending";
  }

  pasteStatusDot.className = `permission-status-dot ${tone}`;
  pasteStatusText.textContent = i18n.t(key);
  pasteStatusText.title = status?.detail || "";
  pasteAuthorizeBtn.hidden = backend !== "wayland_portal" || phase === "ready";
}

pasteAuthorizeBtn.addEventListener("click", async () => {
  pasteAuthorizeBtn.disabled = true;
  renderPasteStatus({ backend: "wayland_portal", phase: "initializing" });
  try {
    renderPasteStatus(await requestPastePermission());
  } catch (error) {
    renderPasteStatus({ backend: "wayland_portal", phase: "denied", detail: String(error) });
  } finally {
    pasteAuthorizeBtn.disabled = false;
  }
});

// 快捷键录制
recordBtn.addEventListener("click", () => {
  (isRecording ? stopRecording() : startRecording()).catch(console.warn);
});

clearBtn.addEventListener("click", () => {
  if (savedConfig) shortcutInput.value = savedConfig.global_shortcut || "";
  shortcutWarning.classList.add("hidden");
  if (isRecording) stopRecording().catch(console.warn);
});

async function startRecording() {
  if (isPinRecording) await stopPinRecording();
  if (isCaptureRecording) await stopCaptureRecording();
  isRecording = true;
  try { await pauseShortcuts(); } catch (e) { console.warn(e); }
  shortcutInput.value = i18n.t("settings.shortcut.recording");
  shortcutInput.classList.add("recording");
  recordBtn.textContent = i18n.t("settings.shortcut.stop");
  shortcutWarning.classList.add("hidden");
  // capture 阶段注册：在录制窗口拿到事件最早期的优先级，
  // 防止冒泡阶段的其他监听器（或浏览器默认行为）抢先处理。
  window.addEventListener("keydown", onKeyDown, { capture: true });
}

async function stopRecording() {
  isRecording = false;
  shortcutInput.classList.remove("recording");
  recordBtn.textContent = i18n.t("settings.shortcut.record");
  window.removeEventListener("keydown", onKeyDown, { capture: true });
  try { await resumeShortcuts(); } catch (e) { console.warn(e); }
  if (shortcutInput.value === i18n.t("settings.shortcut.recording")) {
    shortcutInput.value = savedConfig?.global_shortcut || "";
  }
}

// 修饰键 / e.code → Tauri key 名映射现在在 ./shortcut-recorder.js

async function onKeyDown(e) {
  // capture 阶段先吞掉默认行为和后续监听器
  e.preventDefault();
  e.stopImmediatePropagation();

  const shortcut = keyEventToShortcut(e);
  if (!shortcut) return;

  shortcutInput.value = shortcut;
  // setTimeout(0) 把清理推到下一个事件循环 tick，等当前 keydown 派发完成
  // 后再注销监听器并恢复全局快捷键，避免与异步 IPC 重入。
  setTimeout(() => { stopRecording().catch(console.warn); }, 0);

  try {
    const conflict = await checkShortcutConflict(shortcut);
    conflict ? shortcutWarning.classList.remove("hidden")
             : shortcutWarning.classList.add("hidden");
  } catch (err) { console.warn(err); }
}

// Pin 快捷键录制
pinRecordBtn.addEventListener("click", () => {
  (isPinRecording ? stopPinRecording() : startPinRecording()).catch(console.warn);
});

pinClearBtn.addEventListener("click", () => {
  if (savedConfig) pinShortcutInput.value = savedConfig.pin_shortcut || "Ctrl+2";
  if (isPinRecording) stopPinRecording().catch(console.warn);
});

async function startPinRecording() {
  if (isRecording) await stopRecording();
  if (isCaptureRecording) await stopCaptureRecording();
  isPinRecording = true;
  try { await pauseShortcuts(); } catch (e) { console.warn(e); }
  pinShortcutInput.value = i18n.t("settings.shortcut.recording");
  pinShortcutInput.classList.add("recording");
  pinRecordBtn.textContent = i18n.t("settings.shortcut.stop");
  window.addEventListener("keydown", onPinKeyDown, { capture: true });
}

async function stopPinRecording() {
  isPinRecording = false;
  pinShortcutInput.classList.remove("recording");
  pinRecordBtn.textContent = i18n.t("settings.shortcut.record");
  window.removeEventListener("keydown", onPinKeyDown, { capture: true });
  try { await resumeShortcuts(); } catch (e) { console.warn(e); }
  if (pinShortcutInput.value === i18n.t("settings.shortcut.recording")) {
    pinShortcutInput.value = savedConfig?.pin_shortcut || "Ctrl+2";
  }
}

function onPinKeyDown(e) {
  e.preventDefault();
  e.stopImmediatePropagation();
  const shortcut = keyEventToShortcut(e);
  if (!shortcut) return;
  pinShortcutInput.value = shortcut;
  setTimeout(() => { stopPinRecording().catch(console.warn); }, 0);
}

// Screenshot 快捷键录制
captureRecordBtn.addEventListener("click", () => {
  (isCaptureRecording ? stopCaptureRecording() : startCaptureRecording()).catch(console.warn);
});

captureClearBtn.addEventListener("click", () => {
  if (savedConfig) captureShortcutInput.value = savedConfig.capture_shortcut || "Ctrl+Shift+S";
  if (isCaptureRecording) stopCaptureRecording().catch(console.warn);
});

async function startCaptureRecording() {
  if (isRecording) await stopRecording();
  if (isPinRecording) await stopPinRecording();
  isCaptureRecording = true;
  try { await pauseShortcuts(); } catch (e) { console.warn(e); }
  captureShortcutInput.value = i18n.t("settings.shortcut.recording");
  captureShortcutInput.classList.add("recording");
  captureRecordBtn.textContent = i18n.t("settings.shortcut.stop");
  window.addEventListener("keydown", onCaptureKeyDown, { capture: true });
}

async function stopCaptureRecording() {
  isCaptureRecording = false;
  captureShortcutInput.classList.remove("recording");
  captureRecordBtn.textContent = i18n.t("settings.shortcut.record");
  window.removeEventListener("keydown", onCaptureKeyDown, { capture: true });
  try { await resumeShortcuts(); } catch (e) { console.warn(e); }
  if (captureShortcutInput.value === i18n.t("settings.shortcut.recording")) {
    captureShortcutInput.value = savedConfig?.capture_shortcut || "Ctrl+Shift+S";
  }
}

function onCaptureKeyDown(e) {
  e.preventDefault();
  e.stopImmediatePropagation();
  const shortcut = keyEventToShortcut(e);
  if (!shortcut) return;
  captureShortcutInput.value = shortcut;
  setTimeout(() => { stopCaptureRecording().catch(console.warn); }, 0);
}

// 保存
saveBtn.addEventListener("click", async () => {
  const newShortcut   = shortcutInput.value.trim();
  const newPinShortcut = pinShortcutInput.value.trim();
  const newCaptureShortcut = captureShortcutInput.value.trim();
  const newMaxHistory = parseInt(maxHistoryInput.value, 10) || 0;
  const newLanguage   = languageSelect.value;

  try {
    const newConfig = {
      version: savedConfig?.version ?? 1,
      max_history: newMaxHistory,
      storage_mode: savedConfig?.storage_mode || "persistent",
      global_shortcut: newShortcut || savedConfig?.global_shortcut || "Super+V",
      pin_shortcut: newPinShortcut || savedConfig?.pin_shortcut || "Ctrl+2",
      capture_shortcut: newCaptureShortcut || savedConfig?.capture_shortcut || "Ctrl+Shift+S",
      theme: selectedTheme,
      language: newLanguage,
      delete_confirm_ms: savedConfig?.delete_confirm_ms ?? 1200,
      ocr_result_mode: ocrModeCtrl.value,
      ocr_enabled: ocrToggle.checked,
      tmux_capture: tmuxGroup.style.display !== "none" ? tmuxToggle.checked : (savedConfig?.tmux_capture ?? false),
      auto_paste: autoPasteToggle.checked,
      ...translationSettings.getConfig(),
    };

    await updateConfig(newConfig);
    savedConfig = newConfig;
    showToast(i18n.t("settings.saved"));
  } catch (err) {
    console.error("保存失败:", err);
    showToast(i18n.t("settings.saveFailed", { error: err }));
  }
});

// 检查更新（使用 update-modal 弹窗）
initUpdateModal();
checkUpdateBtn.addEventListener("click", async () => {
  checkUpdateBtn.disabled = true;
  updateStatus.classList.add("hidden");
  updateStatus.classList.remove("error");
  try {
    const found = await checkForUpdate(true);
    if (!found) {
      updateStatus.textContent = i18n.t("settings.about.upToDate");
      updateStatus.classList.remove("hidden");
      setTimeout(() => updateStatus.classList.add("hidden"), 3000);
    }
  } catch (err) {
    console.warn("检查更新失败:", err);
    updateStatus.textContent = i18n.t("settings.about.checkFailed");
    updateStatus.classList.remove("hidden");
    updateStatus.classList.add("error");
    setTimeout(() => updateStatus.classList.add("hidden"), 3000);
  } finally {
    checkUpdateBtn.disabled = false;
  }
});

// 取消
cancelBtn.addEventListener("click", async () => {
  try { await getCurrentWindow().close(); } catch (e) { console.warn(e); }
});

// ── OCR 设置 ──

function updateOcrOptionsVisibility() {
  if (ocrOptions) ocrOptions.style.display = ocrToggle.checked ? "" : "none";
}

ocrToggle.addEventListener("change", updateOcrOptionsVisibility);

async function checkOcrStatus() {
  try {
    const available = await ocrAvailable();
    ocrStatusDot.className = "ocr-status-dot " + (available ? "ocr-ok" : "ocr-missing");
    ocrStatusText.textContent = available
      ? i18n.t("settings.ocr.installed")
      : i18n.t("settings.ocr.notInstalled");
    ocrInstallBtn.style.display = available ? "none" : "";
  } catch (_) {
    ocrStatusDot.className = "ocr-status-dot ocr-missing";
    ocrStatusText.textContent = i18n.t("settings.ocr.notInstalled");
    ocrInstallBtn.style.display = "";
  }
}

ocrInstallBtn.addEventListener("click", async () => {
  ocrInstallBtn.disabled = true;
  ocrInstallBtn.textContent = i18n.t("settings.ocr.installing");
  try {
    await ocrInstall();
    showToast(i18n.t("settings.ocr.installSuccess"));
    await checkOcrStatus();
  } catch (err) {
    // pkexec 用户取消授权时不提示失败
    const msg = String(err?.message || err || "");
    if (!msg.includes("cancelled")) {
      showToast(i18n.t("settings.ocr.installFailed"));
    }
    console.warn("OCR 安装失败:", err);
  } finally {
    ocrInstallBtn.disabled = false;
    ocrInstallBtn.textContent = i18n.t("settings.ocr.install");
  }
});

// 页面加载时检测 OCR 状态
checkOcrStatus();

// ── tmux 捕获 ──

async function checkTmuxAvailability() {
  try {
    const available = await tmuxAvailable();
    if (available) {
      tmuxGroup.style.display = "";
    }
  } catch (_) { /* tmux 不可用，隐藏面板 */ }
}

tmuxToggle.addEventListener("change", async () => {
  try {
    await toggleTmuxCapture(tmuxToggle.checked);
  } catch (err) {
    console.warn("tmux 切换失败:", err);
    tmuxToggle.checked = !tmuxToggle.checked;
    showToast(String(err?.message || err || "tmux error"));
  }
});

checkTmuxAvailability();

// Toast
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

// ── 统计 ──
function fmtSize(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

async function loadStats() {
  try {
    const s = await getStats();
    document.getElementById("stats-total").textContent = s.total;
    document.getElementById("stats-favorites").textContent = s.favorites;
    document.getElementById("stats-text").textContent = s.text_count;
    document.getElementById("stats-html").textContent = s.html_count;
    document.getElementById("stats-image").textContent = s.image_count;
    document.getElementById("stats-size").textContent = fmtSize(s.db_size);
  } catch (e) {
    console.warn("加载统计失败:", e);
  }
}
