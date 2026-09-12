/** 更新弹窗：检查、下载、已安装与失败各自有可达的终态。 */
import { checkUpdate, downloadAndInstallUpdate, getInstallType, openExternalUrl, restartApp } from "./api.ts";
import * as i18n from "../i18n/i18n.js";
const RELEASE_URL = "https://github.com/51hhh/Clippy/releases/latest";
const SKIP_KEY = "skipped_update_version";
const AUTO_UPDATE_INSTALL_TYPES = new Set(["appimage", "windows", "macos"]);
let modal, titleEl, versionEl, bodyEl, progressSection, progressBar, progressText;
let btnSkip, btnLater, btnInstall, btnClose, btnDownload;
let pendingUpdate = null;
let installType = null;
let state = "idle";
let checkGeneration = 0;
let totalBytes = 0;
let receivedBytes = 0;
let previousFocus = null;
const initialized = new WeakSet();

function getElements() {
  modal = document.getElementById("update-modal");
  titleEl = document.getElementById("update-title");
  versionEl = document.getElementById("update-version");
  bodyEl = document.getElementById("update-body");
  progressSection = document.getElementById("update-progress");
  progressBar = document.getElementById("update-progress-bar");
  progressText = document.getElementById("update-progress-text");
  btnSkip = document.getElementById("update-btn-skip");
  btnLater = document.getElementById("update-btn-later");
  btnInstall = document.getElementById("update-btn-install");
  btnClose = document.getElementById("update-btn-close");
  btnDownload = document.getElementById("update-btn-download");
}
function caption(element, key) {
  element.dataset.i18n = key;
  element.textContent = i18n.t(key);
}
function buttons(visible) {
  for (const [name, element] of Object.entries({ skip: btnSkip, later: btnLater, install: btnInstall, close: btnClose, download: btnDownload })) {
    element.classList.toggle("hidden", !visible.includes(name));
    element.disabled = false;
  }
}
function visibleButtons() {
  return [btnInstall, btnLater, btnClose, btnDownload, btnSkip].filter(button => !button.classList.contains("hidden") && !button.disabled);
}
function focusModal() { (visibleButtons()[0] || modal)?.focus(); }
function show() {
  if (!modal) return;
  if (modal.classList.contains("hidden")) previousFocus = document.activeElement;
  modal.classList.remove("hidden");
  focusModal();
}
function hide() {
  modal?.classList.add("hidden");
  if (previousFocus?.isConnected) previousFocus.focus();
}
function message(key) {
  bodyEl.replaceChildren();
  const paragraph = document.createElement("p");
  caption(paragraph, key);
  bodyEl.append(paragraph);
}
function renderChangelog(body) {
  for (const line of body.split("\n").filter(line => line.trim())) {
    const text = line.trim();
    const paragraph = document.createElement("p");
    paragraph.textContent = /^[-*]\s/.test(text) ? `• ${text.slice(2)}` : text.replace(/^#{1,3}\s+/, "");
    if (/^#{1,3}\s/.test(text)) paragraph.style.fontWeight = "600";
    bodyEl.append(paragraph);
  }
}
function showInfoState() {
  state = "available";
  versionEl.textContent = `v${pendingUpdate.version}`;
  caption(titleEl, "update.title");
  caption(btnInstall, "update.install");
  progressSection.classList.add("hidden");
  bodyEl.replaceChildren();
  if (AUTO_UPDATE_INSTALL_TYPES.has(installType)) buttons(["skip", "later", "install"]);
  else { message("update.manualBody"); buttons(["skip", "close", "download"]); }
  renderChangelog(pendingUpdate.body || "");
}
function showDownloadState() {
  state = "installing";
  totalBytes = 0;
  receivedBytes = 0;
  progressSection.classList.remove("hidden");
  buttons([]);
  caption(titleEl, "update.downloading");
  progressBar.style.width = "0%";
  progressText.textContent = "0%";
  focusModal();
}
function showInstalledState(restartFailed = false) {
  state = "installed";
  progressSection.classList.add("hidden");
  if (installType === "windows") {
    // Windows 插件成功启动安装器后会 exit；若宿主返回，只提示安装器交接，不重启旧 exe。
    caption(titleEl, "update.installerTitle");
    message("update.installerBody");
    buttons(["close"]);
  } else {
    caption(titleEl, "update.installedTitle");
    message(restartFailed ? "update.restartFailed" : "update.installedBody");
    caption(btnInstall, "update.restart");
    buttons(["later", "install"]);
  }
  focusModal();
}
function showFailureState() {
  state = "failed";
  caption(titleEl, "update.failedTitle");
  message("update.failedBody");
  progressSection.classList.add("hidden");
  buttons(["close", "download"]);
  focusModal();
}
function onProgress(event) {
  if (state !== "installing") return;
  if (event.total) { totalBytes = event.total; receivedBytes = 0; }
  if (event.chunkLength) receivedBytes += event.chunkLength;
  if (totalBytes > 0) {
    const percent = Math.min(100, Math.round((receivedBytes / totalBytes) * 100));
    progressBar.style.width = `${percent}%`;
    progressText.textContent = `${percent}%`;
  }
}
async function doInstall() {
  if (!pendingUpdate || state !== "available") return;
  showDownloadState();
  try {
    await downloadAndInstallUpdate(pendingUpdate.update, onProgress);
    showInstalledState();
  } catch (error) {
    console.warn("自动更新失败:", error);
    showFailureState();
  }
}
async function doRestart() {
  if (state !== "installed" || installType === "windows" || btnInstall.disabled) return;
  btnInstall.disabled = true;
  try { await restartApp(); }
  catch (error) { console.warn("重启失败:", error); showInstalledState(true); }
}

export async function checkForUpdate(manual = false) {
  getElements();
  // 已替换的应用等待重启时，不能再次下载安装同一份文件。
  if (state === "installing" || state === "installed") { show(); return true; }
  const generation = ++checkGeneration;
  try {
    const result = await checkUpdate();
    if (generation !== checkGeneration || !result?.available) return false;
    if (!manual && localStorage.getItem(SKIP_KEY) === result.version) return false;
    const detectedType = await getInstallType();
    if (generation !== checkGeneration) return false;
    pendingUpdate = result;
    installType = detectedType;
    showInfoState();
    show();
    return true;
  } catch (error) {
    console.warn("检查更新失败:", error);
    if (manual) throw error;
    return false;
  }
}
export function initUpdateModal() {
  getElements();
  if (!modal || initialized.has(modal)) return;
  initialized.add(modal);
  modal.setAttribute("role", "dialog");
  modal.setAttribute("aria-modal", "true");
  modal.setAttribute("aria-labelledby", "update-title");
  modal.tabIndex = -1;
  modal.addEventListener("keydown", event => {
    if (event.key === "Escape" && state !== "installing") {
      event.preventDefault();
      hide();
    } else if (event.key === "Tab") {
      const controls = [...modal.querySelectorAll("button")].filter(button => !button.classList.contains("hidden") && !button.disabled);
      const index = controls.indexOf(document.activeElement);
      if (index < 0 || (!event.shiftKey && index === controls.length - 1) || (event.shiftKey && index === 0)) {
        event.preventDefault();
        (controls[event.shiftKey ? controls.length - 1 : 0] || modal).focus();
      }
    }
  });
  btnSkip.addEventListener("click", () => {
    if (pendingUpdate) localStorage.setItem(SKIP_KEY, pendingUpdate.version);
    hide();
  });
  btnLater.addEventListener("click", hide);
  btnClose.addEventListener("click", hide);
  btnInstall.addEventListener("click", () => { void (state === "installed" ? doRestart() : doInstall()); });
  btnDownload.addEventListener("click", () => { void openExternalUrl(RELEASE_URL).then(hide).catch(console.warn); });
}
