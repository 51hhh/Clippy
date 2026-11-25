/**
 * pin-project-drop.js — 主窗口的可编辑 PNG 工程拖放入口
 *
 * 常态界面不放导入按钮：只有原生拖放进入时才展示提示。前端只做单文件 PNG 的
 * 快速筛选，iTXt 是否为有效 Clippy 工程始终由后端在文件信任边界重新验证。
 */

import { onCurrentWindowDragDrop, openPinProjectFile } from "./api.ts";
import { t } from "../i18n/i18n.js";

function isSinglePng(paths) {
  return paths.length === 1 && /\.png$/i.test(paths[0]);
}

function createOverlay() {
  const overlay = document.createElement("div");
  overlay.className = "pin-project-drop-overlay";
  overlay.setAttribute("role", "status");
  overlay.setAttribute("aria-live", "polite");
  overlay.setAttribute("aria-atomic", "true");
  overlay.textContent = t("pinProject.dropHint");
  overlay.hidden = true;
  document.body.appendChild(overlay);
  return overlay;
}

/**
 * 安装当前窗口的原生拖放监听，返回卸载函数。
 *
 * 后端会再次验证 PNG 和 Clippy iTXt，所以普通 PNG 永远不会作为通用图片导入。
 */
export async function initPinProjectDrop({
  subscribe = onCurrentWindowDragDrop,
  openProject = openPinProjectFile,
  createDropOverlay = createOverlay,
} = {}) {
  const overlay = createDropOverlay();
  let hideTimer = null;
  const setVisible = (visible) => {
    overlay.hidden = !visible;
  };
  const cancelScheduledHide = () => {
    if (hideTimer === null) return;
    clearTimeout(hideTimer);
    hideTimer = null;
  };
  const announce = (key) => {
    cancelScheduledHide();
    overlay.textContent = t(key);
    setVisible(true);
    hideTimer = setTimeout(() => {
      hideTimer = null;
      setVisible(false);
    }, 1800);
  };

  const handleDragDrop = (event) => {
    if (event.type === "enter" || event.type === "over") {
      // i18n 可在设置保存后热切换；覆盖层不是常驻节点，进入时再取一次当前语言。
      cancelScheduledHide();
      overlay.textContent = t("pinProject.dropHint");
      setVisible(true);
      return;
    }
    if (event.type === "leave") {
      cancelScheduledHide();
      setVisible(false);
      return;
    }
    if (event.type !== "drop") return;

    setVisible(false);
    if (!isSinglePng(event.paths)) {
      console.info(t("pinProject.dropRejected"));
      announce("pinProject.dropRejected");
      return;
    }

    void openProject(event.paths[0]).then((label) => {
      // `null` 是后端已安全拒绝普通 PNG / 坏 iTXt 的结果，不能伪装成打开成功。
      if (label === null) {
        console.info(t("pinProject.dropRejected"));
        announce("pinProject.dropRejected");
      }
    }).catch((error) => {
      console.warn(t("pinProject.openFailed"), error);
      announce("pinProject.openFailed");
    });
  };

  let unlisten;
  try {
    unlisten = await subscribe(handleDragDrop);
  } catch (error) {
    // 订阅失败时初始化没有成功，不能把不可达的隐藏覆盖层留在主窗口 DOM。
    cancelScheduledHide();
    overlay.remove();
    throw error;
  }

  return () => {
    cancelScheduledHide();
    unlisten();
    overlay.remove();
  };
}

export const __test__ = { isSinglePng };
