const FALLBACK_STATUS = {
  backend: "copy_only",
  phase: "unavailable",
};

/** 把后端状态映射为稳定的展示状态，便于独立测试所有平台分支。 */
export function describePasteStatus(status) {
  const backend = status?.backend || FALLBACK_STATUS.backend;
  const phase = status?.phase || FALLBACK_STATUS.phase;

  if (backend === "x11") {
    return { i18nKey: "settings.autoPaste.x11Ready", tone: "ready", showAuthorize: false };
  }
  if (backend === "wayland_portal" && phase === "ready") {
    return { i18nKey: "settings.autoPaste.portalReady", tone: "ready", showAuthorize: false };
  }
  if (backend === "wayland_portal" && phase === "initializing") {
    return { i18nKey: "settings.autoPaste.initializing", tone: "pending", showAuthorize: true };
  }
  if (backend === "wayland_portal" && phase === "denied") {
    return { i18nKey: "settings.autoPaste.denied", tone: "unavailable", showAuthorize: true };
  }
  if (backend === "wayland_portal") {
    return {
      i18nKey: "settings.autoPaste.permissionRequired",
      tone: "pending",
      showAuthorize: true,
    };
  }
  return {
    i18nKey: "settings.autoPaste.unavailable",
    tone: "unavailable",
    showAuthorize: false,
  };
}

export function createPastePermissionController({
  statusDot,
  statusText,
  authorizeButton,
  getStatus,
  requestPermission,
  translate,
}) {
  let currentStatus = null;

  function render(status) {
    currentStatus = status;
    const view = describePasteStatus(status);
    statusDot.className = `permission-status-dot ${view.tone}`;
    statusText.textContent = translate(view.i18nKey);
    statusText.title = status?.detail || "";
    authorizeButton.hidden = !view.showAuthorize;
  }

  async function load() {
    try {
      render(await getStatus());
    } catch (error) {
      render({ ...FALLBACK_STATUS, detail: String(error) });
    }
  }

  async function authorize() {
    authorizeButton.disabled = true;
    render({ backend: "wayland_portal", phase: "initializing" });
    try {
      render(await requestPermission());
    } catch (error) {
      render({ backend: "wayland_portal", phase: "denied", detail: String(error) });
    } finally {
      authorizeButton.disabled = false;
    }
  }

  authorizeButton.addEventListener("click", () => {
    void authorize();
  });

  return {
    load,
    render,
    refreshLabels() {
      if (currentStatus) render(currentStatus);
    },
  };
}
