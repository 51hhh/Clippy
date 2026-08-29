/**
 * app.js — 主窗口入口 + 键盘路由
 */

import * as theme         from "./theme.js";
import * as clipboardList from "./clipboard-react-facade.js";
import * as previewPanel  from "./preview-panel.js";
import * as codec         from "./codec.js";
import * as i18n          from "../i18n/i18n.js";
import { initUpdateModal, checkForUpdate } from "./update-modal.js";
import {
  getConfig, getClips, onClipAdded, onClipRemoved, onConfigChanged,
  hideCurrentWindow, onShortcutRegisterFailed, onPinCurrent, pinClip,
} from "./api.ts";
import "../styles/themes.css";
import "../styles/base.css";
import "../styles/components.css";
import { mountClipboardWorkspace, mountTranslationPanel } from "../react/main/mount";
import { translationStore } from "../react/main/translationStore";
import { createKeyboardRouter } from "./keyboard-router.js";

function whenReady(fn) {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", fn);
  } else {
    fn();
  }
}

whenReady(async () => {
  let config;
  try {
    config = await getConfig();
  } catch (err) {
    console.warn("配置加载失败:", err);
    config = { theme: "light", language: "auto" };
  }

  theme.applyTheme(config.theme || "light");
  i18n.init(config.language || "auto");

  mountClipboardWorkspace(document.getElementById("clipboard-react-root"));
  mountTranslationPanel(document.getElementById("translation-react-root"));
  previewPanel.init({
    onVisibilityChange: (visible) => translationStore.setPanelVisible(visible),
  });
  translationStore.setConfig(config);
  codec.init();

  clipboardList.init({
    onFocusChange: (clip) => {
      previewPanel.updatePreview(clip);
      translationStore.setClip(clip);
    },
  });

  await clipboardList.refresh();

  await onClipAdded((clip) => {
    console.debug("[clip-added]", clip.id, clip.content_type, clip.byte_size);
    clipboardList.markDirty();
    clipboardList.prependClip(clip);
  });
  await onClipRemoved((id) => {
    clipboardList.markDirty();
    clipboardList.removeClip(id);
  });
  await onConfigChanged((newConfig) => {
    theme.applyTheme(newConfig.theme || "light");
    i18n.init(newConfig.language || "auto");
    clipboardList.refreshLabels();
    translationStore.setConfig(newConfig);
  });

  await onShortcutRegisterFailed((failure) => {
    // 主窗口只记日志；可操作的提示由设置页负责（它能读到存量失败记录）。
    console.warn(
      `快捷键 [${failure.action}] "${failure.shortcut}" 在 ${failure.session} 会话注册失败：${failure.reason}`,
    );
  });

  await onPinCurrent(async () => {
    // 系统快捷键触发：pin 焦点条目，若列表空则从后端获取最新条目
    let clip = clipboardList.getFocusedClip() || clipboardList.getLatestClip();
    if (!clip) {
      const clips = await getClips(null, false, 0, 1);
      clip = clips[0] || null;
    }
    if (clip) pinClip(clip.id).catch(err => console.warn("Pin 失败:", err));
  });

  // 初始化更新弹窗并自动检查
  initUpdateModal();
  checkForUpdate(false).catch(console.warn);

  const keyboardRouter = createKeyboardRouter({
    clipboardList,
    previewPanel,
    codec,
    pinClip,
    hidePanel: tryHidePanel,
    // 翻译面板的动作以适配器注入，路由不直接依赖 React store（保持可单测）
    translation: { translate: () => translationStore.translate() },
  });

  window.addEventListener("keydown", keyboardRouter.onKeyDown);
  window.addEventListener("focus", onWindowFocus);
  window.addEventListener("blur", onWindowBlur);
});

function tryHidePanel() {
  void hideCurrentWindow();
}

async function onWindowFocus() {
  console.debug("[focus] dirty=", clipboardList.isDirty());
  // 仅在有新数据时才全量刷新，否则只恢复渲染
  if (clipboardList.isDirty()) {
    await clipboardList.refresh();
  } else {
    clipboardList.restoreRender();
  }
}

function onWindowBlur() {
  if (previewPanel.isVisible() || codec.isVisible()) return; // 面板打开时不隐藏窗口
  clipboardList.releaseMemory();
  previewPanel.clearContent();
  translationStore.clear();
}
