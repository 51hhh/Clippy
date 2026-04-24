/**
 * settings.js — 设置面板逻辑
 * 独立于主窗口，通过 Tauri IPC 读写配置、录制快捷键。
 */

const { invoke } = window.__TAURI__.core;

// ── DOM 引用 ──────────────────────────────────────────────────────────────────

const shortcutInput    = document.getElementById("shortcut-input");
const recordBtn        = document.getElementById("shortcut-record-btn");
const clearBtn         = document.getElementById("shortcut-clear-btn");
const shortcutHint     = document.getElementById("shortcut-hint");
const shortcutWarning  = document.getElementById("shortcut-warning");
const themeSelect      = document.getElementById("theme-select");
const maxHistoryInput  = document.getElementById("max-history-input");
const saveBtn          = document.getElementById("save-btn");
const cancelBtn        = document.getElementById("cancel-btn");
const toast            = document.getElementById("toast");

// ── 状态 ──────────────────────────────────────────────────────────────────────

/** 从后端加载的原始配置（用于重置和对比变更） */
let savedConfig = null;

/** 是否处于快捷键录制模式 */
let isRecording = false;

// ── 初始化 ─────────────────────────────────────────────────────────────────────

document.addEventListener("DOMContentLoaded", async () => {
  try {
    savedConfig = await invoke("get_config");
    fillForm(savedConfig);
    applyTheme(savedConfig.theme || "light");
  } catch (err) {
    console.error("加载配置失败:", err);
  }
});

/**
 * 用配置值填充表单控件。
 * @param {AppConfig} config
 */
function fillForm(config) {
  shortcutInput.value   = config.global_shortcut || "";
  themeSelect.value     = config.theme || "light";
  maxHistoryInput.value = config.max_history ?? 100;
}

/**
 * 设置文档主题。
 * @param {string} theme
 */
function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
}

// ── 主题实时预览 ───────────────────────────────────────────────────────────────

themeSelect.addEventListener("change", () => {
  applyTheme(themeSelect.value);
});

// ── 快捷键录制 ─────────────────────────────────────────────────────────────────

recordBtn.addEventListener("click", () => {
  if (isRecording) {
    stopRecording();
  } else {
    startRecording();
  }
});

clearBtn.addEventListener("click", () => {
  // 恢复到已保存的快捷键
  if (savedConfig) {
    shortcutInput.value = savedConfig.global_shortcut || "";
  }
  shortcutWarning.classList.add("hidden");
  stopRecording();
});

function startRecording() {
  isRecording = true;
  shortcutInput.value = "Press keys...";
  shortcutInput.classList.add("recording");
  recordBtn.textContent = "Stop";
  shortcutWarning.classList.add("hidden");
  document.addEventListener("keydown", onKeyDown);
}

function stopRecording() {
  isRecording = false;
  shortcutInput.classList.remove("recording");
  recordBtn.textContent = "Record";
  document.removeEventListener("keydown", onKeyDown);
  // 如果用户没有按到有效组合，恢复原值
  if (shortcutInput.value === "Press keys...") {
    shortcutInput.value = savedConfig ? savedConfig.global_shortcut : "";
  }
}

/**
 * 将 keydown 事件转为 Tauri 快捷键格式字符串。
 * 格式示例："CmdOrCtrl+Shift+V"、"Alt+Space"、"Super+V"
 * @param {KeyboardEvent} e
 * @returns {string|null} — 仅修饰键按下时返回 null
 */
function keyEventToShortcut(e) {
  const modifiers = [];
  if (e.ctrlKey)  modifiers.push("CmdOrCtrl");
  if (e.altKey)   modifiers.push("Alt");
  if (e.shiftKey) modifiers.push("Shift");
  if (e.metaKey)  modifiers.push("Super");

  // 仅修饰键则忽略
  const modifierKeys = ["Control", "Alt", "Shift", "Meta", "OS"];
  if (modifierKeys.includes(e.key)) return null;

  // 必须至少有一个修饰键
  if (modifiers.length === 0) return null;

  let key = e.key;
  if (key === " ")          key = "Space";
  else if (key.length === 1) key = key.toUpperCase();

  return [...modifiers, key].join("+");
}

/**
 * keydown 事件处理：录制快捷键组合。
 * @param {KeyboardEvent} e
 */
async function onKeyDown(e) {
  e.preventDefault();
  e.stopPropagation();

  const shortcut = keyEventToShortcut(e);
  if (!shortcut) return; // 仅修饰键，继续等待

  shortcutInput.value = shortcut;
  stopRecording();

  // 冲突检测
  try {
    const conflict = await invoke("check_shortcut_conflict", { shortcut });
    if (conflict) {
      shortcutWarning.classList.remove("hidden");
    } else {
      shortcutWarning.classList.add("hidden");
    }
  } catch (err) {
    console.warn("快捷键冲突检测失败:", err);
  }
}

// ── 保存 ───────────────────────────────────────────────────────────────────────

saveBtn.addEventListener("click", async () => {
  const newShortcut   = shortcutInput.value.trim();
  const newTheme      = themeSelect.value;
  const newMaxHistory = parseInt(maxHistoryInput.value, 10) || 0;

  try {
    // 如果快捷键有变动，先动态更新快捷键
    if (savedConfig && newShortcut !== savedConfig.global_shortcut && newShortcut) {
      await invoke("update_shortcut", { newShortcut });
    }

    // 构建完整配置并保存
    const newConfig = {
      max_history:     newMaxHistory,
      storage_mode:    savedConfig ? savedConfig.storage_mode : "persistent",
      global_shortcut: newShortcut || (savedConfig ? savedConfig.global_shortcut : "Super+V"),
      theme:           newTheme,
    };

    await invoke("update_config", { newConfig });

    // 更新本地已保存的配置快照
    savedConfig = newConfig;

    showToast("Settings saved!");
  } catch (err) {
    console.error("保存配置失败:", err);
    showToast("Save failed: " + err);
  }
});

// ── 取消 ───────────────────────────────────────────────────────────────────────

cancelBtn.addEventListener("click", async () => {
  try {
    // 使用 Tauri window API 关闭当前窗口
    const { getCurrentWindow } = window.__TAURI__.window;
    await getCurrentWindow().close();
  } catch (err) {
    // 回退：尝试通过 invoke 关闭
    console.warn("关闭窗口失败:", err);
  }
});

// ── Toast 通知 ──────────────────────────────────────────────────────────────────

/**
 * 显示底部通知条，自动在 2 秒后消失。
 * @param {string} message
 */
function showToast(message) {
  toast.textContent = message;
  toast.classList.remove("hidden");
  // 触发 reflow 以保证过渡动画生效
  void toast.offsetWidth;
  toast.classList.add("show");

  setTimeout(() => {
    toast.classList.remove("show");
    // 等待淡出动画完成后再隐藏
    setTimeout(() => {
      toast.classList.add("hidden");
    }, 300);
  }, 2000);
}
