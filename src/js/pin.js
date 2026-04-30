/**
 * pin.js — 贴图窗口逻辑
 * 从 URL 参数获取 clip ID，加载内容并渲染。
 * 支持：拖拽移动、滚轮缩放、Ctrl+滚轮透明度、双击关闭、右键菜单。
 */

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { PhysicalPosition } from "@tauri-apps/api/dpi";

const params = new URLSearchParams(window.location.search);
const clipId = Number(params.get("id"));
const container = document.getElementById("pin-container");
const content = document.getElementById("pin-content");
const menu = document.getElementById("pin-context-menu");
const appWindow = getCurrentWindow();

// ── 状态 ──
let scale = 1;
let opacity = 1;
let locked = false;
let baseWidth = 400;
let baseHeight = 300;
let cachedScaleFactor = 1;

// ── 初始化：加载内容 ──
async function init() {
  if (!clipId) return;
  cachedScaleFactor = await appWindow.scaleFactor();
  try {
    const clip = await invoke("get_clip_detail", { id: clipId });
    if (clip.content_type === "image") {
      const base64 = await invoke("get_clip_image", { id: clipId });
      if (base64) {
        const img = document.createElement("img");
        img.src = `data:image/png;base64,${base64}`;
        img.alt = "pinned image";
        img.draggable = false;
        content.appendChild(img);
        // 图片加载后调整窗口为原始大小
        img.onload = () => {
          const w = img.naturalWidth;
          const h = img.naturalHeight;
          baseWidth = w;
          baseHeight = h;
          appWindow.setSize(new LogicalSize(w, h));
        };
      }
    } else {
      // 文本类型
      const pre = document.createElement("pre");
      pre.textContent = clip.text_content || "";
      content.appendChild(pre);
    }
  } catch (err) {
    content.textContent = "Failed to load content";
    console.error("贴图加载失败:", err);
  }
  // 强制置顶（某些 WM 可能忽略创建时的 always_on_top）
  await appWindow.setAlwaysOnTop(true);
}

// ── 滚轮缩放 / Ctrl+滚轮透明度 ──
container.addEventListener("wheel", async (e) => {
  e.preventDefault();
  if (e.ctrlKey) {
    // 调节透明度
    opacity = Math.max(0.1, Math.min(1, opacity + (e.deltaY > 0 ? -0.05 : 0.05)));
    container.style.opacity = opacity;
  } else {
    // 缩放：直接调整窗口大小，保持中心位置不变
    const step = 0.05;
    const oldScale = scale;
    scale = Math.max(0.2, Math.min(5, scale + (e.deltaY > 0 ? -step : step)));
    const oldW = Math.round(baseWidth * oldScale);
    const oldH = Math.round(baseHeight * oldScale);
    const newW = Math.round(baseWidth * scale);
    const newH = Math.round(baseHeight * scale);
    // 位移补偿：窗口中心保持不动
    const pos = await appWindow.outerPosition();
    const dx = Math.round((oldW - newW) / 2 * cachedScaleFactor);
    const dy = Math.round((oldH - newH) / 2 * cachedScaleFactor);
    await appWindow.setSize(new LogicalSize(newW, newH));
    await appWindow.setPosition(new PhysicalPosition(pos.x + dx, pos.y + dy));
  }
}, { passive: false });

// ── 拖拽移动（替代 data-tauri-drag-region，避免事件冲突）──
container.addEventListener("mousedown", (e) => {
  if (e.button === 0 && !locked) {
    appWindow.startDragging();
  }
});

// ── 双击关闭 ──
container.addEventListener("dblclick", () => {
  appWindow.close();
});

// ── 右键菜单 ──
container.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  menu.hidden = false;
  menu.style.left = `${e.clientX}px`;
  menu.style.top = `${e.clientY}px`;
  // 更新锁定按钮文字
  const lockBtn = menu.querySelector('[data-action="lock"]');
  if (lockBtn) lockBtn.textContent = locked ? "Unlock Position" : "Lock Position";
});

document.addEventListener("click", () => {
  menu.hidden = true;
});

menu.addEventListener("click", async (e) => {
  const action = e.target.dataset.action;
  if (!action) return;
  menu.hidden = true;
  if (action === "copy") {
    await invoke("select_clip", { id: clipId });
  } else if (action === "lock") {
    locked = !locked;
  } else if (action === "close") {
    appWindow.close();
  }
});

init();
