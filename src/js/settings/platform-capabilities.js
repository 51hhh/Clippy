/** 设置页的平台能力摘要。后端 PlatformInfo 是唯一事实源，前端只负责翻译和展示。 */

const CAPABILITY_NAMES = {
  clipboard_text: "clipboardText",
  clipboard_image: "clipboardImage",
  auto_paste: "autoPaste",
  global_shortcuts: "globalShortcuts",
  screen_capture: "screenCapture",
  window_pick: "windowPick",
  absolute_window_position: "absoluteWindowPosition",
  always_on_top: "alwaysOnTop",
  ocr: "ocr",
};

const PORTAL_INTERFACES = {
  global_shortcuts: "GlobalShortcuts",
  remote_desktop: "RemoteDesktop",
  screenshot: "Screenshot",
  screen_cast: "ScreenCast",
};

const STATE_TONES = {
  available: "ready",
  permission_required: "pending",
  degraded: "pending",
  unsupported: "unavailable",
};

export function describePlatformEnvironment(platform, translate) {
  const parts = [
    translate(`settings.platform.os.${platform.operating_system}`),
    translate(`settings.platform.session.${platform.session}`),
    platform.desktop_environment,
    platform.architecture,
  ].filter(Boolean);
  if (platform.xwayland_available) parts.push("XWayland");
  return parts.join(" · ");
}

export function describePortalInterfaces(platform, translate) {
  if (platform.operating_system !== "linux") {
    return translate("settings.platform.portalNotApplicable");
  }
  if (!platform.portal.desktop_service_available) {
    return translate("settings.platform.portalServiceUnavailable");
  }
  const interfaces = Object.entries(PORTAL_INTERFACES).map(([key, name]) => {
    const item = platform.portal[key];
    return item.available ? `${name} v${item.version}` : `${name} —`;
  });
  return `${translate("settings.platform.portalLabel")}: ${interfaces.join(" · ")}`;
}

export function createPlatformCapabilities({ summary, portal, list, translate }) {
  function render(platform) {
    summary.textContent = describePlatformEnvironment(platform, translate);
    portal.textContent = describePortalInterfaces(platform, translate);
    list.replaceChildren();

    for (const [key, capability] of Object.entries(platform.capabilities)) {
      const row = document.createElement("div");
      row.className = `platform-capability ${STATE_TONES[capability.state] || "unavailable"}`;
      row.setAttribute("role", "listitem");

      const name = document.createElement("span");
      name.className = "platform-capability-name";
      name.textContent = translate(`settings.platform.capability.${CAPABILITY_NAMES[key] || key}`);

      const status = document.createElement("span");
      status.className = "platform-capability-state";
      status.textContent = translate(`settings.platform.state.${capability.state}`);

      row.append(name, status);
      if (capability.reason) {
        const reason = document.createElement("span");
        reason.className = "platform-capability-reason";
        reason.textContent = translate(`settings.platform.reason.${capability.reason}`);
        row.append(reason);
      }
      list.append(row);
    }
  }

  function renderError() {
    summary.textContent = translate("settings.platform.loadFailed");
    portal.textContent = "";
    list.replaceChildren();
  }

  return { render, renderError };
}
