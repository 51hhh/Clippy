import { describe, expect, it, vi } from "vitest";
import { createOcrSettings } from "../js/settings/ocr-settings.js";

function setup(available = false) {
  document.body.innerHTML = `
    <input id="toggle" type="checkbox">
    <span id="dot"></span>
    <span id="status"></span>
    <button id="install"></button>
    <div id="options"></div>
  `;
  const modeControl = { value: "preview" };
  const controller = createOcrSettings({
    toggle: document.querySelector("#toggle"),
    statusDot: document.querySelector("#dot"),
    statusText: document.querySelector("#status"),
    installButton: document.querySelector("#install"),
    options: document.querySelector("#options"),
    modeControl,
    checkAvailable: vi.fn().mockResolvedValue(available),
    install: vi.fn().mockResolvedValue("ok"),
    translate: (key) => key,
    showToast: vi.fn(),
  });
  return {
    controller,
    installButton: document.querySelector("#install"),
    statusText: document.querySelector("#status"),
  };
}

describe("OCR 设置", () => {
  it("非 Linux 平台缺少 Tesseract 时隐藏应用内安装按钮", async () => {
    const { controller, installButton } = setup(false);
    await controller.checkStatus();
    expect(installButton.hidden).toBe(true);
  });

  it("仅在 Linux 明确支持安装且 Tesseract 缺失时显示按钮", async () => {
    const { controller, installButton } = setup(false);
    controller.setPlatform("linux");
    await controller.checkStatus();
    expect(installButton.hidden).toBe(false);
  });

  it("已经安装 Tesseract 时始终隐藏安装按钮", async () => {
    const { controller, installButton } = setup(true);
    controller.setPlatform("linux");
    await controller.checkStatus();
    expect(installButton.hidden).toBe(true);
  });

  it.each([
    ["linux", "settings.ocr.notInstalledLinux"],
    ["windows", "settings.ocr.notInstalledWindows"],
    ["macos", "settings.ocr.notInstalledMacos"],
    ["other", "settings.ocr.notInstalled"],
  ])("%s 缺少 Tesseract 时显示对应安装提示", async (platform, key) => {
    const { controller, statusText } = setup(false);
    controller.setPlatform(platform);
    await controller.checkStatus();
    expect(statusText.textContent).toBe(key);
  });
});
