export function createOcrSettings({
  toggle,
  statusDot,
  statusText,
  installButton,
  options,
  modeControl,
  checkAvailable,
  install,
  translate,
  showToast,
}) {
  let platform = "unknown";
  let installSupported = false;

  function missingStatusKey() {
    const keys = {
      linux: "settings.ocr.notInstalledLinux",
      windows: "settings.ocr.notInstalledWindows",
      macos: "settings.ocr.notInstalledMacos",
    };
    return keys[platform] || "settings.ocr.notInstalled";
  }

  function updateOptionsVisibility() {
    options.hidden = !toggle.checked;
  }

  async function checkStatus() {
    let available = false;
    try {
      available = await checkAvailable();
    } catch (_) {
      // 查询失败与未安装使用相同的可恢复界面。
    }
    statusDot.className = `ocr-status-dot ${available ? "ocr-ok" : "ocr-missing"}`;
    statusText.textContent = translate(
      available ? "settings.ocr.installed" : missingStatusKey(),
    );
    installButton.hidden = available || !installSupported;
  }

  async function installOcr() {
    installButton.disabled = true;
    installButton.textContent = translate("settings.ocr.installing");
    try {
      await install();
      showToast(translate("settings.ocr.installSuccess"));
      await checkStatus();
    } catch (error) {
      const message = String(error?.message || error || "");
      if (!message.includes("cancelled")) {
        showToast(translate("settings.ocr.installFailed"));
      }
      console.warn("OCR 安装失败:", error);
    } finally {
      installButton.disabled = false;
      installButton.textContent = translate("settings.ocr.install");
    }
  }

  toggle.addEventListener("change", updateOptionsVisibility);
  installButton.addEventListener("click", () => {
    void installOcr();
  });

  return {
    checkStatus,
    setPlatform(operatingSystem) {
      platform = operatingSystem || "unknown";
      installSupported = platform === "linux";
      if (!installSupported) installButton.hidden = true;
    },
    fill(config) {
      modeControl.value = config.ocr_result_mode || "preview";
      toggle.checked = config.ocr_enabled !== false;
      updateOptionsVisibility();
    },
    getConfig() {
      return {
        ocr_result_mode: modeControl.value,
        ocr_enabled: toggle.checked,
      };
    },
  };
}
