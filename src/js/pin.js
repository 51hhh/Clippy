/**
 * pin.js — 贴图窗口逻辑
 * 从 URL 参数获取 clip ID，加载内容并渲染。
 * 支持：拖拽移动、滚轮缩放、Ctrl+滚轮透明度、双击关闭、右键菜单。
 */

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

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

// ── 初始化：加载内容 ──
async function init() {
  if (!clipId) return;
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
        // 图片加载后调整窗口大小
        img.onload = () => {
          const w = Math.min(img.naturalWidth + 16, 800);
          const h = Math.min(img.naturalHeight + 16, 600);
          appWindow.setSize(new (window.__TAURI__.window.LogicalSize)(w, h));
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
}

// ── 滚轮缩放 / Ctrl+滚轮透明度 ──
container.addEventListener("wheel", (e) => {
  e.preventDefault();
  if (e.ctrlKey) {
    // 调节透明度
    opacity = Math.max(0.1, Math.min(1, opacity + (e.deltaY > 0 ? -0.05 : 0.05)));
    container.style.opacity = opacity;
  } else {
    // 缩放内容
    scale = Math.max(0.2, Math.min(5, scale + (e.deltaY > 0 ? -0.1 : 0.1)));
    content.style.transform = `scale(${scale})`;
  }
}, { passive: false });

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
    // 锁定时移除拖拽区域
    if (locked) {
      container.removeAttribute("data-tauri-drag-region");
    } else {
      container.setAttribute("data-tauri-drag-region", "");
    }
  } else if (action === "close") {
    appWindow.close();
  }
});

init();
