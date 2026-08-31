/**
 * 截图辅助服务（GNOME Shell 扩展）的安装、卸载与状态显示。
 *
 * 为什么需要这么一个"服务"：GNOME Wayland 下客户端既拿不到窗口的屏幕坐标，
 * 也没法在没有窗口聚焦时通过 Portal 截图（那个授权对话框只允许聚焦的应用弹），
 * 而这两件事 gnome-shell 进程自己都做得到，所以 Clippy 附带一个小扩展。
 * 装完必须注销一次才生效——Shell 不会热扫描新的扩展目录，这一点必须明确告诉用户，
 * 否则会以为"装了没用"。同理，Clippy 升级后扩展文件是新的、跑着的还是旧的
 * （stale），也要单独说清楚。
 */

/** 把后端状态映射成展示状态，便于把所有分支都单测到。 */
export function describeWindowProbe(status) {
  if (!status?.supported) {
    return {
      tone: "neutral",
      stateKey: "settings.windowProbe.stateUnsupported",
      detailKey: "settings.windowProbe.detailUnsupported",
      showInstall: false,
      showUninstall: false,
    };
  }
  if (status.active && status.stale) {
    // 文件已经升级过，跑着的还是上次登录时加载的旧版：窗口速选照旧可用，
    // 新增能力（扩展截图）要等注销一次。这条不能说成"已就绪"。
    return {
      tone: "pending",
      stateKey: "settings.windowProbe.statePendingUpdate",
      detailKey: "settings.windowProbe.detailPendingUpdate",
      showInstall: false,
      showUninstall: true,
    };
  }
  if (status.active) {
    return {
      tone: "ready",
      stateKey: "settings.windowProbe.stateActive",
      detailKey: "settings.windowProbe.detailActive",
      showInstall: false,
      showUninstall: true,
    };
  }
  if (status.installed && !status.userExtensionsEnabled) {
    // 装了、也登记了，但用户在系统层面把所有扩展关掉了——再点安装也没用。
    return {
      tone: "pending",
      stateKey: "settings.windowProbe.stateBlocked",
      detailKey: "settings.windowProbe.detailBlocked",
      showInstall: false,
      showUninstall: true,
    };
  }
  if (status.installed) {
    return {
      tone: "pending",
      stateKey: "settings.windowProbe.statePendingLogout",
      detailKey: "settings.windowProbe.detailPendingLogout",
      showInstall: false,
      showUninstall: true,
    };
  }
  return {
    tone: "unavailable",
    stateKey: "settings.windowProbe.stateMissing",
    detailKey: "settings.windowProbe.detailMissing",
    showInstall: true,
    // 目录没了但 gsettings 里还挂着条目时，也得留一个清理入口。
    showUninstall: Boolean(status.enabled),
  };
}

export function createWindowProbeCard({
  card,
  dot,
  stateText,
  detailText,
  installButton,
  uninstallButton,
  recheckButton,
  getStatus,
  install,
  uninstall,
  translate,
  notify,
}) {
  let currentStatus = null;

  function render(status) {
    currentStatus = status;
    const view = describeWindowProbe(status);
    card.className = `service-card ${view.tone}`;
    dot.className = `service-card-dot ${view.tone}`;
    stateText.textContent = translate(view.stateKey);
    detailText.textContent = translate(view.detailKey);
    installButton.hidden = !view.showInstall;
    uninstallButton.hidden = !view.showUninstall;
  }

  function setBusy(busy) {
    for (const button of [installButton, uninstallButton, recheckButton]) {
      button.disabled = busy;
    }
  }

  async function load() {
    try {
      render(await getStatus());
    } catch (error) {
      render({ supported: true, installed: false, enabled: false, active: false });
      detailText.textContent = String(error);
    }
  }

  /** 安装动作由用户显式触发，应用从不擅自往用户的 GNOME 里塞扩展。 */
  async function runInstall() {
    setBusy(true);
    try {
      const outcome = await install();
      render(outcome.status);
      notify?.(
        translate(
          outcome.needsLogout
            ? "settings.windowProbe.installedNeedsLogout"
            : "settings.windowProbe.installedActive",
        ),
      );
    } catch (error) {
      await load();
      notify?.(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function runUninstall() {
    setBusy(true);
    try {
      render(await uninstall());
      notify?.(translate("settings.windowProbe.uninstalled"));
    } catch (error) {
      await load();
      notify?.(String(error));
    } finally {
      setBusy(false);
    }
  }

  installButton.addEventListener("click", () => void runInstall());
  uninstallButton.addEventListener("click", () => void runUninstall());
  recheckButton.addEventListener("click", () => void load());

  return {
    load,
    render,
    refreshLabels() {
      if (currentStatus) render(currentStatus);
    },
  };
}
