/** 更新弹窗：检查、下载、已安装与失败各自有可达的终态。 */
import { checkUpdate, downloadAndInstallUpdate, getAppUpdateState, onAppUpdateState, openExternalUrl, restartApp } from "./api.ts";
import * as i18n from "../i18n/i18n.js";
const RELEASE_URL = "https://github.com/51hhh/Clippy/releases/latest";
const SKIP_KEY = "skipped_update_version";
const AUTO_UPDATE_INSTALL_TYPES = new Set(["appimage", "windows", "macos"]);
const defaultServices = { checkUpdate, downloadAndInstallUpdate, getAppUpdateState, onAppUpdateState, openExternalUrl, restartApp };

/** 生产与 dev 审阅共用同一控制器；审阅只注入合成服务，不替换全局 IPC。 */
export function createUpdateModal({ rootDocument = globalThis.document, services = defaultServices, translate = i18n.t } = {}) {
  const document = rootDocument;
  const localStorage = document.defaultView.localStorage;
  let snapshot = null;
  let initialization = null;
  let unlisten = null;
  let disposed = false;
  let pollTimer = null;
  let installPending = false;
  let modal, titleEl, versionEl, bodyEl, progressSection, progressBar, progressText;
  let btnSkip, btnLater, btnInstall, btnClose, btnDownload;
  let pendingUpdate = null;
  let installType = null;
  let state = "idle";
  let checkGeneration = 0;
  let previousFocus = null;


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
    element.textContent = translate(key);
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
    if (!modal || modal.classList.contains("hidden")) return;
    modal.classList.add("hidden");
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
    bodyEl.replaceChildren();
    renderChangelog(pendingUpdate?.body || "");
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
  async function doInstall() {
    if (!pendingUpdate || state !== "available" || installPending) return;
    const requestedRevision = snapshot?.revision;
    const version = pendingUpdate.version;
    installPending = true;
    showDownloadState();
    try {
      applySnapshot(await services.downloadAndInstallUpdate(version));
    } catch (error) {
      console.warn("自动更新失败:", error);
      // 新的原生快照已说明任务去向，旧请求错误不能把它降级为失败/未知。
      if (disposed || snapshot?.revision !== requestedRevision) return;
      // 应答丢失不代表后台安装失败。先查权威状态，不能贸然再次安装。
      try {
        applySnapshot(await services.getAppUpdateState());
        if (!disposed && snapshot?.status === "available" && snapshot.version === version) showFailureState();
      }
      catch { showReadFailure(requestedRevision); }
    } finally { installPending = false; }
  }
  async function doRestart() {
    if (state !== "installed" || installType === "windows" || btnInstall.disabled) return;
    btnInstall.disabled = true;
    try { await services.restartApp(); }
    catch (error) { console.warn("重启失败:", error); showInstalledState(true); }
  }

  function showUnknownState() {
    state = "unknown";
    caption(titleEl, "update.statusUnknownTitle");
    message("update.statusUnknownBody");
    progressSection.classList.add("hidden");
    buttons(["close"]);
    focusModal();
  }
  function showReadFailure(requestedRevision) {
    if (!disposed && snapshot?.revision === requestedRevision) showUnknownState();
  }
  function renderSnapshot() {
    if (!snapshot) return;
    pendingUpdate = snapshot.version ? { version: snapshot.version, body: snapshot.body } : null;
    versionEl.textContent = snapshot.version ? `v${snapshot.version}` : "";
    installType = snapshot.install_type;
    state = snapshot.status;
    if (state === "idle") {
      bodyEl.replaceChildren();
      progressSection.classList.add("hidden");
      buttons([]);
      hide();
    } else if (state === "available") showInfoState();
    else if (state === "installing") {
      if (progressSection.classList.contains("hidden")) showDownloadState();
      const percent = snapshot.total ? Math.min(100, Math.round(snapshot.downloaded / snapshot.total * 100)) : 0;
      progressBar.style.width = `${percent}%`;
      progressText.textContent = `${percent}%`;
    } else if (state === "installed") showInstalledState();
    else if (state === "failed") showFailureState();
  }
  function applySnapshot(next) {
    if (disposed || !next || (snapshot && next.revision < snapshot.revision)) return;
    snapshot = next;
    pendingUpdate = next.version ? { version: next.version, body: next.body } : null;
    installType = next.install_type;
    state = next.status;
    if (state === "idle" || !modal.classList.contains("hidden")) renderSnapshot();
    if (pollTimer !== null) clearTimeout(pollTimer);
    pollTimer = null;
    // 事件是即时通知，查询是恢复兜底。丢一条完成通知不会让窗口永久停在 Downloading。
    if (state === "installing") pollTimer = setTimeout(async () => {
      pollTimer = null;
      const requestedRevision = snapshot?.revision;
      try { applySnapshot(await services.getAppUpdateState()); }
      catch { showReadFailure(requestedRevision); }
    }, 2000);
  }
  async function checkForUpdate(manual = false) {
    await initUpdateModal();
    const generation = ++checkGeneration;
    const result = await services.checkUpdate().catch(error => {
      console.warn("检查更新失败:", error);
      if (manual) throw error;
      return null;
    });
    if (disposed || generation !== checkGeneration || !result) return false;
    applySnapshot(result);
    if (state === "idle") return false;
    if (!manual && state === "available" && localStorage.getItem(SKIP_KEY) === pendingUpdate?.version) return false;
    renderSnapshot();
    show();
    return true;
  }
  function initUpdateModal() {
    if (initialization) return initialization;
    getElements();
    if (!modal) return Promise.resolve();
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
    btnDownload.addEventListener("click", () => { void services.openExternalUrl(RELEASE_URL).then(hide).catch(console.warn); });
    initialization = (async () => {
      try {
        const stop = await services.onAppUpdateState(applySnapshot);
        if (disposed) stop(); else unlisten = stop;
      } catch (error) { console.warn("订阅更新状态失败:", error); }
      try { applySnapshot(await services.getAppUpdateState()); }
      catch (error) { console.warn("读取更新状态失败:", error); }
    })();
    return initialization;
  }
  return { initUpdateModal, checkForUpdate, dispose() {
    disposed = true;
    unlisten?.();
    if (pollTimer !== null) clearTimeout(pollTimer);
  } };
}
let controller;
export function initUpdateModal() {
  controller ||= createUpdateModal();
  return controller.initUpdateModal();
}
export function checkForUpdate(manual = false) {
  controller ||= createUpdateModal();
  return controller.checkForUpdate(manual);
}
