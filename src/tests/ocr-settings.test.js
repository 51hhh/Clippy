import { describe, expect, it, vi } from "vitest";
import { createOcrSettings, describeOcrStatus } from "../js/settings/ocr-settings.js";

const unavailable = {
  available: false,
  activeEngine: "unavailable",
  enhancedState: "not_configured",
  configurationSource: "none",
  pipelineId: null,
  runtimeIdentity: null,
  modelIdentities: [],
  tesseractAvailable: false,
  fallbackReason: null,
  issueCode: null,
  missingItems: ["manifest"],
};

const tesseract = {
  ...unavailable,
  available: true,
  activeEngine: "tesseract",
  tesseractAvailable: true,
};

const enhanced = {
  available: true,
  activeEngine: "ppocrv6+edgegnn",
  enhancedState: "ready",
  configurationSource: "settings",
  pipelineId: "ppocrv6-edgegnn-v1",
  runtimeIdentity: "ppocrv6-edgegnn-v1:abc",
  modelIdentities: [
    { role: "det", sha256: "a".repeat(64) },
    { role: "rec", sha256: "b".repeat(64) },
  ],
  tesseractAvailable: true,
  fallbackReason: null,
  issueCode: null,
  missingItems: [],
};

function setup(status = unavailable) {
  document.body.innerHTML = `
    <input id="toggle" type="checkbox">
    <div id="card"><span id="dot"></span><span id="state"></span><p id="detail"></p></div>
    <span id="engine"></span><div id="pipeline-row"><span id="pipeline"></span></div>
    <span id="fallback"></span><input id="manifest">
    <button id="browse"></button><button id="clear"></button>
    <button id="recheck"></button><button id="install"></button>
    <div id="options"></div>
  `;
  const modeControl = { value: "preview" };
  const getStatus = vi.fn().mockResolvedValue(status);
  const pickManifest = vi.fn().mockResolvedValue(null);
  const showToast = vi.fn();
  const controller = createOcrSettings({
    toggle: document.querySelector("#toggle"),
    card: document.querySelector("#card"),
    statusDot: document.querySelector("#dot"),
    statusText: document.querySelector("#state"),
    detailText: document.querySelector("#detail"),
    engineText: document.querySelector("#engine"),
    pipelineRow: document.querySelector("#pipeline-row"),
    pipelineText: document.querySelector("#pipeline"),
    fallbackText: document.querySelector("#fallback"),
    manifestInput: document.querySelector("#manifest"),
    browseButton: document.querySelector("#browse"),
    clearButton: document.querySelector("#clear"),
    recheckButton: document.querySelector("#recheck"),
    installButton: document.querySelector("#install"),
    options: document.querySelector("#options"),
    modeControl,
    getStatus,
    pickManifest,
    install: vi.fn().mockResolvedValue("ok"),
    translate: key => key,
    showToast,
  });
  return {
    controller,
    getStatus,
    pickManifest,
    showToast,
    installButton: document.querySelector("#install"),
    statusText: document.querySelector("#state"),
    detailText: document.querySelector("#detail"),
    manifestInput: document.querySelector("#manifest"),
    pipelineRow: document.querySelector("#pipeline-row"),
    pipelineText: document.querySelector("#pipeline"),
  };
}

describe("OCR 设置", () => {
  it("非 Linux 平台缺少 Tesseract 时隐藏应用内安装按钮", async () => {
    const { controller, installButton } = setup();
    await controller.checkStatus();
    expect(installButton.hidden).toBe(true);
  });

  it("仅在 Linux 明确支持安装且 Tesseract 缺失时显示按钮", async () => {
    const { controller, installButton } = setup();
    controller.setPlatform("linux");
    await controller.checkStatus();
    expect(installButton.hidden).toBe(false);
  });

  it("Tesseract 可用时始终隐藏安装按钮", async () => {
    const { controller, installButton } = setup(tesseract);
    controller.setPlatform("linux");
    await controller.checkStatus();
    expect(installButton.hidden).toBe(true);
  });

  it.each([
    ["linux", "settings.ocr.notInstalledLinux"],
    ["windows", "settings.ocr.notInstalledWindows"],
    ["macos", "settings.ocr.notInstalledMacos"],
    ["other", "settings.ocr.notInstalled"],
  ])("%s 缺少全部 OCR 引擎时显示对应安装提示", async (platform, key) => {
    const { controller, detailText } = setup();
    controller.setPlatform(platform);
    await controller.checkStatus();
    expect(detailText.textContent).toBe(key);
  });

  it("有效增强运行时显示管线与模型身份", async () => {
    const { controller, statusText, pipelineRow, pipelineText } = setup(enhanced);
    await controller.checkStatus();
    expect(statusText.textContent).toBe("settings.ocr.enhancedReady");
    expect(pipelineRow.hidden).toBe(false);
    expect(pipelineText.textContent).toContain("ppocrv6-edgegnn-v1");
    expect(pipelineText.textContent).toContain("det:aaaaaaaaaa");
  });

  it("坏模型显示稳定原因和具体缺失角色，同时保留 Tesseract 回退", async () => {
    const bad = {
      ...tesseract,
      enhancedState: "invalid",
      fallbackReason: "enhanced_configuration_invalid",
      issueCode: "model_hash_mismatch",
      missingItems: ["edge"],
    };
    const { controller, statusText, detailText } = setup(bad);
    await controller.checkStatus();
    expect(statusText.textContent).toBe("settings.ocr.fallbackActive");
    expect(detailText.textContent).toContain("settings.ocr.issueModelHash");
    expect(detailText.textContent).toContain("settings.ocr.missingLayoutModel");
  });

  it("选择 manifest 后先按候选路径检查，保存配置时带上该路径", async () => {
    const fixture = setup(enhanced);
    fixture.pickManifest.mockResolvedValue("/opt/clippy-ocr/manifest.json");
    document.querySelector("#browse").click();
    await vi.waitFor(() => expect(fixture.getStatus).toHaveBeenCalledWith("/opt/clippy-ocr/manifest.json"));
    expect(fixture.manifestInput.value).toBe("/opt/clippy-ocr/manifest.json");
    expect(fixture.controller.getConfig()).toMatchObject({
      enhanced_ocr_manifest_path: "/opt/clippy-ocr/manifest.json",
    });
  });

  it("文件选择失败可恢复且明确通知用户", async () => {
    const fixture = setup();
    fixture.pickManifest.mockRejectedValue(new Error("dialog unavailable"));
    document.querySelector("#browse").click();
    await vi.waitFor(() => expect(fixture.showToast).toHaveBeenCalledWith("settings.ocr.browseFailed"));
    expect(document.querySelector("#browse").disabled).toBe(false);
  });

  it("状态映射不会把无增强配置的 Tesseract 说成完整增强链", () => {
    expect(describeOcrStatus(tesseract).stateKey).toBe("settings.ocr.tesseractReady");
    expect(describeOcrStatus(enhanced).stateKey).toBe("settings.ocr.enhancedReady");
  });
});
