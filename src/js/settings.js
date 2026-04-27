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
  isDevBinary,
  pauseShortcuts,
  resumeShortcuts,
  updateConfig,
  updateShortcut,
} from "./api.js";
import { keyEventToShortcut } from "./shortcut-recorder.js";
import { initUpdateModal, checkForUpdate } from "./update-modal.js";
import * as i18n from "../i18n/i18n.js";
import "../styles/themes.css";
import "../styles/base.css";
import "../styles/settings.css";

// DOM 引用
const shortcutInput   = document.getElementById("shortcut-input");
const recordBtn       = document.getElementById("shortcut-record-btn");
const clearBtn        = document.getElementById("shortcut-clear-btn");
const shortcutWarning = document.getElementById("shortcut-warning");
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
        autostartToggle.disabled = true;
        autostartToggle.title = "Autostart is disabled for development builds";
        // 顺手清理已被错误写入的 dev 路径自启项
        try { await disableAutostart(); } catch (e) { console.warn("清理 dev 自启项失败:", e); }
      } else {
        autostartToggle.checked = await isAutostartEnabled();
      }
    } catch (e) { console.warn("获取自启动状态失败:", e); }
  } catch (err) {
    console.error("加载配置失败:", err);
    selectedTheme = "light";
    renderThemeGrid();
    i18n.init("auto");
  }
});

function fillForm(config) {
  shortcutInput.value   = config.global_shortcut || "";
  maxHistoryInput.value = config.max_history ?? 100;
  languageSelect.value  = config.language || "auto";
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

// 快捷键录制
recordBtn.addEventListener("click", () => {
  isRecording ? stopRecording() : startRecording();
});

clearBtn.addEventListener("click", () => {
  if (savedConfig) shortcutInput.value = savedConfig.global_shortcut || "";
  shortcutWarning.classList.add("hidden");
  if (isRecording) stopRecording().catch(console.warn);
});

async function startRecording() {
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

// 保存
saveBtn.addEventListener("click", async () => {
  const newShortcut   = shortcutInput.value.trim();
  const newMaxHistory = parseInt(maxHistoryInput.value, 10) || 0;
  const newLanguage   = languageSelect.value;

  try {
    if (savedConfig && newShortcut && newShortcut !== savedConfig.global_shortcut) {
      await updateShortcut(newShortcut);
    }

    const newConfig = {
      max_history: newMaxHistory,
      storage_mode: savedConfig?.storage_mode || "persistent",
      global_shortcut: newShortcut || savedConfig?.global_shortcut || "Super+V",
      theme: selectedTheme,
      language: newLanguage,
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
