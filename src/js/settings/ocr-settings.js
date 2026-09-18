const ISSUE_KEYS = Object.freeze({
  manifest_path_invalid: "settings.ocr.issueManifestPath",
  manifest_missing: "settings.ocr.issueManifestMissing",
  manifest_format_invalid: "settings.ocr.issueManifestFormat",
  manifest_contract_invalid: "settings.ocr.issueManifestContract",
  python_missing: "settings.ocr.issuePython",
  script_missing: "settings.ocr.issueScript",
  parameters_invalid: "settings.ocr.issueParameters",
  model_missing: "settings.ocr.issueModelMissing",
  model_identity_invalid: "settings.ocr.issueModelIdentity",
  model_hash_mismatch: "settings.ocr.issueModelHash",
  model_too_large: "settings.ocr.issueModelSize",
  model_unreadable: "settings.ocr.issueModelUnreadable",
  runtime_module_missing: "settings.ocr.issueRuntimeModule",
  runtime_invalid: "settings.ocr.issueRuntime",
});

const MISSING_KEYS = Object.freeze({
  manifest: "settings.ocr.missingManifest",
  python: "settings.ocr.missingPython",
  script: "settings.ocr.missingScript",
  det: "settings.ocr.missingDetector",
  rec: "settings.ocr.missingRecognizer",
  dictionary: "settings.ocr.missingDictionary",
  edge: "settings.ocr.missingLayoutModel",
  englishRec: "settings.ocr.missingRecognizer",
  englishDictionary: "settings.ocr.missingDictionary",
  "pipeline.py": "settings.ocr.missingPipelineModule",
  "edge_features.py": "settings.ocr.missingPipelineModule",
  "layout_groups.py": "settings.ocr.missingPipelineModule",
  "visual_paragraphs.py": "settings.ocr.missingPipelineModule",
});

export function describeOcrStatus(status, platform = "unknown") {
  const missingKey = {
    linux: "settings.ocr.notInstalledLinux",
    windows: "settings.ocr.notInstalledWindows",
    macos: "settings.ocr.notInstalledMacos",
  }[platform] || "settings.ocr.notInstalled";

  if (!status) {
    return {
      tone: "unavailable",
      stateKey: "settings.ocr.checkFailed",
      detailKey: "settings.ocr.checkFailedDetail",
      issueKey: null,
      missingKey,
    };
  }
  if (status.enhancedState === "ready") {
    return {
      tone: "ready",
      stateKey: "settings.ocr.enhancedReady",
      detailKey: status.configurationSource === "environment"
        ? "settings.ocr.enhancedEnvironmentDetail"
        : "settings.ocr.enhancedReadyDetail",
      issueKey: null,
      missingKey,
    };
  }
  if (status.enhancedState === "invalid") {
    return {
      tone: status.tesseractAvailable ? "pending" : "unavailable",
      stateKey: status.tesseractAvailable
        ? "settings.ocr.fallbackActive"
        : "settings.ocr.enhancedInvalid",
      detailKey: "settings.ocr.enhancedInvalidDetail",
      issueKey: ISSUE_KEYS[status.issueCode] || "settings.ocr.issueRuntime",
      missingKey,
    };
  }
  if (status.tesseractAvailable) {
    return {
      tone: "pending",
      stateKey: "settings.ocr.tesseractReady",
      detailKey: "settings.ocr.tesseractReadyDetail",
      issueKey: null,
      missingKey,
    };
  }
  return {
    tone: "unavailable",
    stateKey: "settings.ocr.unavailable",
    detailKey: missingKey,
    issueKey: null,
    missingKey,
  };
}

export function createOcrSettings({
  toggle,
  card,
  statusDot,
  statusText,
  detailText,
  engineText,
  pipelineRow,
  pipelineText,
  fallbackText,
  manifestInput,
  browseButton,
  clearButton,
  recheckButton,
  installButton,
  options,
  modeControl,
  getStatus,
  pickManifest,
  install,
  translate,
  showToast,
}) {
  let platform = "unknown";
  let installSupported = false;
  let currentStatus = null;

  function updateOptionsVisibility() {
    options.hidden = !toggle.checked;
  }

  function setBusy(busy) {
    for (const button of [browseButton, clearButton, recheckButton, installButton]) {
      button.disabled = busy;
    }
  }

  function render(status) {
    currentStatus = status;
    const view = describeOcrStatus(status, platform);
    card.className = `service-card ${view.tone}`;
    statusDot.className = `service-card-dot ${view.tone}`;
    statusText.textContent = translate(view.stateKey);
    const details = [translate(view.detailKey)];
    if (view.issueKey) details.push(translate(view.issueKey));
    const missing = (status?.missingItems || [])
      .map(item => translate(MISSING_KEYS[item] || "settings.ocr.missingUnknown"));
    if (missing.length && status?.enhancedState === "invalid") {
      details.push(`${translate("settings.ocr.missingPrefix")}: ${[...new Set(missing)].join(", ")}`);
    }
    detailText.textContent = details.join(" ");
    engineText.textContent = translate(
      status?.activeEngine === "ppocrv6+edgegnn"
        ? "settings.ocr.engineEnhanced"
        : status?.activeEngine === "tesseract"
          ? "settings.ocr.engineTesseract"
          : "settings.ocr.engineUnavailable",
    );
    const modelSummary = (status?.modelIdentities || [])
      .map(model => `${model.role}:${model.sha256.slice(0, 10)}`)
      .join(" · ");
    pipelineRow.hidden = !status?.pipelineId;
    pipelineText.textContent = status?.pipelineId
      ? `${status.pipelineId} · ${modelSummary}`
      : "—";
    fallbackText.textContent = translate(
      status?.tesseractAvailable
        ? "settings.ocr.fallbackReady"
        : "settings.ocr.fallbackMissing",
    );
    installButton.hidden = Boolean(status?.tesseractAvailable) || !installSupported;
    clearButton.disabled = !manifestInput.value;
  }

  async function checkStatus() {
    setBusy(true);
    try {
      render(await getStatus(manifestInput.value));
    } catch (error) {
      render(null);
      console.warn("OCR 状态检查失败:", error);
    } finally {
      setBusy(false);
      clearButton.disabled = !manifestInput.value;
    }
  }

  async function chooseManifest() {
    setBusy(true);
    try {
      const selected = await pickManifest();
      if (selected) {
        manifestInput.value = selected;
        await checkStatus();
      }
    } catch (error) {
      showToast(translate("settings.ocr.browseFailed"));
      console.warn("OCR manifest 选择失败:", error);
    } finally {
      setBusy(false);
      clearButton.disabled = !manifestInput.value;
    }
  }

  async function installOcr() {
    setBusy(true);
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
      setBusy(false);
      installButton.textContent = translate("settings.ocr.install");
    }
  }

  toggle.addEventListener("change", updateOptionsVisibility);
  browseButton.addEventListener("click", () => void chooseManifest());
  clearButton.addEventListener("click", () => {
    manifestInput.value = "";
    void checkStatus();
  });
  recheckButton.addEventListener("click", () => void checkStatus());
  installButton.addEventListener("click", () => void installOcr());

  return {
    checkStatus,
    setPlatform(operatingSystem) {
      platform = operatingSystem || "unknown";
      installSupported = platform === "linux";
      if (currentStatus) render(currentStatus);
    },
    fill(config) {
      modeControl.value = config.ocr_result_mode || "preview";
      toggle.checked = config.ocr_enabled !== false;
      manifestInput.value = config.enhanced_ocr_manifest_path || "";
      updateOptionsVisibility();
    },
    getConfig() {
      return {
        ocr_result_mode: modeControl.value,
        ocr_enabled: toggle.checked,
        enhanced_ocr_manifest_path: manifestInput.value,
      };
    },
    refreshLabels() {
      if (currentStatus) render(currentStatus);
    },
  };
}
