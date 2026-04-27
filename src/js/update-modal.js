/**
 * update-modal.js — 更新弹窗逻辑（主窗口 modal）
 * 三态：信息态（版本+changelog+按钮）、下载态（进度条）、回退态（引导下载链接）
 */

import {
  checkUpdate,
  downloadAndInstallUpdate,
  getAppVersion,
  getInstallType,
  openExternalUrl,
} from "./api.js";
import * as i18n from "../i18n/i18n.js";

const RELEASE_URL = "https://github.com/51hhh/Clippy/releases/latest";
const SKIP_KEY = "skipped_update_version";

// DOM 引用（延迟获取，等 DOM 就绪）
let modal, titleEl, versionEl, bodyEl, progressSection, progressBar, progressText;
let btnSkip, btnLater, btnInstall, btnClose, btnDownload;

function getElements() {
  modal           = document.getElementById("update-modal");
  titleEl         = document.getElementById("update-title");
  versionEl       = document.getElementById("update-version");
  bodyEl          = document.getElementById("update-body");
  progressSection = document.getElementById("update-progress");
  progressBar     = document.getElementById("update-progress-bar");
  progressText    = document.getElementById("update-progress-text");
  btnSkip         = document.getElementById("update-btn-skip");
  btnLater        = document.getElementById("update-btn-later");
  btnInstall      = document.getElementById("update-btn-install");
  btnClose        = document.getElementById("update-btn-close");
  btnDownload     = document.getElementById("update-btn-download");
}

let pendingUpdate = null;
let totalBytes = 0;
let receivedBytes = 0;

/** 显示 modal */
function show() {
  if (modal) modal.classList.remove("hidden");
}

/** 隐藏 modal */
function hide() {
  if (modal) modal.classList.add("hidden");
}

/** 切换到信息态 */
function showInfoState(version, body) {
  versionEl.textContent = `v${version}`;
  // 安全渲染 changelog：按行创建元素，识别 markdown 列表项
  bodyEl.replaceChildren();
  const lines = body.split("\n").filter((l) => l.trim());
  for (const line of lines) {
    const trimmed = line.trim();
    // markdown 列表项：- 或 * 开头
    if (/^[-*]\s/.test(trimmed)) {
      const li = document.createElement("p");
      li.textContent = `• ${trimmed.slice(2)}`;
      li.style.paddingLeft = "8px";
      bodyEl.appendChild(li);
    } else if (/^#{1,3}\s/.test(trimmed)) {
      // markdown 标题：粗体显示
      const h = document.createElement("p");
      h.textContent = trimmed.replace(/^#+\s*/, "");
      h.style.fontWeight = "600";
      h.style.color = "var(--text-primary)";
      bodyEl.appendChild(h);
    } else {
      const p = document.createElement("p");
      p.textContent = trimmed;
      bodyEl.appendChild(p);
    }
  }
  titleEl.textContent = i18n.t("update.title");
  progressSection.classList.add("hidden");
  btnSkip.classList.remove("hidden");
  btnLater.classList.remove("hidden");
  btnInstall.classList.remove("hidden");
  btnClose.classList.add("hidden");
  btnDownload.classList.add("hidden");
}

/** 切换到下载态 */
function showDownloadState() {
  progressSection.classList.remove("hidden");
  btnSkip.classList.add("hidden");
  btnLater.classList.add("hidden");
  btnInstall.classList.add("hidden");
  btnClose.classList.add("hidden");
  btnDownload.classList.add("hidden");
  titleEl.textContent = i18n.t("update.downloading");
  progressBar.style.width = "0%";
  progressText.textContent = "0%";
}

/** 切换到回退态（下载失败） */
function showFallbackState() {
  titleEl.textContent = i18n.t("update.fallbackTitle");
  bodyEl.replaceChildren();
  const p = document.createElement("p");
  p.textContent = i18n.t("update.fallbackBody");
  bodyEl.appendChild(p);
  progressSection.classList.add("hidden");
  btnSkip.classList.add("hidden");
  btnLater.classList.add("hidden");
  btnInstall.classList.add("hidden");
  btnClose.classList.remove("hidden");
  btnDownload.classList.remove("hidden");
}

/** deb 安装态：展示版本+changelog，但只提供 Skip / 手动下载 */
function showDebUpdateState(version, body) {
  versionEl.textContent = `v${version}`;
  // 渲染 changelog（复用 showInfoState 的安全渲染逻辑）
  bodyEl.replaceChildren();
  const hint = document.createElement("p");
  hint.textContent = i18n.t("update.fallbackBody");
  hint.style.color = "var(--text-secondary)";
  hint.style.marginBottom = "8px";
  bodyEl.appendChild(hint);

  const lines = body.split("\n").filter((l) => l.trim());
  for (const line of lines) {
    const trimmed = line.trim();
    if (/^[-*]\s/.test(trimmed)) {
      const li = document.createElement("p");
      li.textContent = `• ${trimmed.slice(2)}`;
      li.style.paddingLeft = "8px";
      bodyEl.appendChild(li);
    } else if (/^#{1,3}\s/.test(trimmed)) {
      const h = document.createElement("p");
      h.textContent = trimmed.replace(/^#+\s*/, "");
      h.style.fontWeight = "600";
      h.style.color = "var(--text-primary)";
      bodyEl.appendChild(h);
    } else {
      const p = document.createElement("p");
      p.textContent = trimmed;
      bodyEl.appendChild(p);
    }
  }
  titleEl.textContent = i18n.t("update.title");
  progressSection.classList.add("hidden");
  btnSkip.classList.remove("hidden");
  btnLater.classList.add("hidden");
  btnInstall.classList.add("hidden");
  btnClose.classList.add("hidden");
  btnDownload.classList.remove("hidden");
}

/** 更新进度 */
function onProgress(event) {
  if (event.total) {
    totalBytes = event.total;
    receivedBytes = 0;
  }
  if (event.chunkLength) {
    receivedBytes += event.chunkLength;
  }
  if (totalBytes > 0) {
    const pct = Math.min(100, Math.round((receivedBytes / totalBytes) * 100));
    progressBar.style.width = `${pct}%`;
    progressText.textContent = `${pct}%`;
  }
}

/** 执行安装 */
async function doInstall() {
  if (!pendingUpdate) return;
  showDownloadState();
  try {
    await downloadAndInstallUpdate(pendingUpdate.update, onProgress);
    // Tauri 自动重启，无需额外处理
  } catch (err) {
    console.warn("自动更新失败，切换回退模式:", err);
    showFallbackState();
  }
}

/**
 * 检查更新
 * @param {boolean} manual - 是否手动触发（手动模式不检查 skip 记忆）
 * @returns {Promise<boolean>} 是否有可用更新
 */
export async function checkForUpdate(manual = false) {
  getElements();
  try {
    const result = await checkUpdate();
    if (!result || !result.available) return false;

    // 非手动模式：检查用户是否跳过了此版本
    if (!manual) {
      const skipped = localStorage.getItem(SKIP_KEY);
      if (skipped === result.version) return false;
    }

    pendingUpdate = result;

    // 检测安装方式：仅 AppImage 支持自动更新
    const installType = await getInstallType();
    if (installType !== "appimage") {
      // deb / dev：展示版本和 changelog，但只提供手动下载
      showDebUpdateState(result.version, result.body);
    } else {
      showInfoState(result.version, result.body);
    }
    show();
    return true;
  } catch (err) {
    console.warn("检查更新失败:", err);
    return false;
  }
}

/** 初始化事件绑定 */
export function initUpdateModal() {
  getElements();
  if (!modal) return;

  btnSkip.addEventListener("click", () => {
    if (pendingUpdate) {
      localStorage.setItem(SKIP_KEY, pendingUpdate.version);
    }
    hide();
  });

  btnLater.addEventListener("click", () => {
    hide();
  });

  btnInstall.addEventListener("click", () => {
    doInstall().catch(console.warn);
  });

  btnClose.addEventListener("click", () => {
    hide();
  });

  btnDownload.addEventListener("click", () => {
    openExternalUrl(RELEASE_URL).catch(console.warn);
    hide();
  });
}
