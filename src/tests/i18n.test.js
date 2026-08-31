/**
 * i18n.js — 国际化模块测试
 */
import { readdirSync, readFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { describe, it, expect, beforeEach } from "vitest";
import * as i18n from "../i18n/i18n.js";
import { describeWindowProbe } from "../js/settings/window-probe.js";
import en from "../i18n/en.json";
import zhCN from "../i18n/zh-CN.json";

const root = resolve(process.cwd());

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return /\.(?:js|ts|tsx)$/.test(entry.name) ? [path] : [];
  });
}

describe("i18n", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("init 设置中文 locale 后 t() 返回中文", () => {
    i18n.init("zh-CN");
    expect(i18n.t("settings.title")).toBe("设置");
    expect(i18n.t("settings.save")).toBe("保存");
  });

  it("init 设置英文 locale 后 t() 返回英文", () => {
    i18n.init("en");
    expect(i18n.t("settings.title")).toBe("Settings");
    expect(i18n.t("settings.save")).toBe("Save");
  });

  it("不存在的 key 返回 key 本身", () => {
    i18n.init("en");
    expect(i18n.t("nonexistent.key")).toBe("nonexistent.key");
  });

  it("支持参数插值", () => {
    i18n.init("zh-CN");
    expect(i18n.t("time.minutesAgo", { n: 5 })).toBe("5 分钟前");
    i18n.init("en");
    expect(i18n.t("time.minutesAgo", { n: 3 })).toBe("3 min ago");
  });

  it("settings.tmux.* key 在中文和英文中都存在", () => {
    i18n.init("zh-CN");
    expect(i18n.t("settings.tmux.label")).toBe("Tmux 捕获");
    expect(i18n.t("settings.tmux.hint")).toBe("通过 hook 捕获 tmux copy-mode 缓冲区内容");
    i18n.init("en");
    expect(i18n.t("settings.tmux.label")).toBe("Tmux Capture");
    expect(i18n.t("settings.tmux.hint")).toBe("Capture tmux copy-mode buffer via hook");
  });

  it("settings.captureShortcut.* key 在中文和英文中都存在", () => {
    i18n.init("zh-CN");
    expect(i18n.t("settings.captureShortcut.label")).toBe("截图快捷键");
    expect(i18n.t("settings.captureShortcut.hint")).toBe("启动截图的快捷键");
    i18n.init("en");
    expect(i18n.t("settings.captureShortcut.label")).toBe("Screenshot Shortcut");
    expect(i18n.t("settings.captureShortcut.hint")).toBe("Shortcut to start a screenshot");
  });

  it("settings.stats.* key 在中文和英文中都存在", () => {
    i18n.init("zh-CN");
    expect(i18n.t("settings.stats.label")).toBe("统计");
    expect(i18n.t("settings.stats.total")).toBe("总计");
    expect(i18n.t("settings.stats.favorites")).toBe("收藏");
    expect(i18n.t("settings.stats.text")).toBe("文本");
    expect(i18n.t("settings.stats.html")).toBe("富文本");
    expect(i18n.t("settings.stats.image")).toBe("图片");
    expect(i18n.t("settings.stats.dbSize")).toBe("数据库大小");
    i18n.init("en");
    expect(i18n.t("settings.stats.label")).toBe("Statistics");
    expect(i18n.t("settings.stats.total")).toBe("Total");
    expect(i18n.t("settings.stats.dbSize")).toBe("DB Size");
  });

  it("settings.translation.* key 在中文和英文中都存在", () => {
    i18n.init("zh-CN");
    expect(i18n.t("settings.translation.label")).toBe("翻译");
    expect(i18n.t("settings.translation.providerOpenAI")).toBe("OpenAI 兼容服务");
    expect(i18n.t("settings.translation.privacy")).toContain("图片绝不会上传");
    i18n.init("en");
    expect(i18n.t("settings.translation.label")).toBe("Translation");
    expect(i18n.t("settings.translation.providerLibre")).toBe("LibreTranslate-compatible");
    expect(i18n.t("settings.translation.privacy")).toContain("Images are never uploaded");
  });

  it("applyToDOM 替换 data-i18n 元素文本", () => {
    document.body.innerHTML = '<span data-i18n="settings.title">Settings</span>';
    i18n.init("zh-CN");
    expect(document.querySelector("[data-i18n]").textContent).toBe("设置");
  });

  it("applyToDOM 替换 data-i18n-attr 指定的属性", () => {
    document.body.innerHTML = '<input data-i18n="settings.shortcut.placeholder" data-i18n-attr="placeholder" placeholder="Click">';
    i18n.init("zh-CN");
    expect(document.querySelector("input").getAttribute("placeholder")).toBe("点击录制以设置...");
  });

  it("auto locale 解析中文浏览器为 zh-CN", () => {
    // navigator.language 默认是 en 在 jsdom，但 init("auto") 应回退到 en
    i18n.init("auto");
    expect(i18n.currentLocale()).toBe("en");
  });

  it("中英文 locale key 集合完全一致", () => {
    expect(Object.keys(zhCN).sort()).toEqual(Object.keys(en).sort());
  });

  // 缺 key 不报错，只会把 key 本身显示在界面上（`t()` 的兜底就是返回 key）。
  // codec 面板一次加了二十多个字段名，靠肉眼核对必然漏，这里整源码扫一遍。
  it("源码里写死的 i18n key 在两个 locale 里都存在", () => {
    const files = sourceFiles(resolve(root, "js")).concat(sourceFiles(resolve(root, "react")));
    const missing = new Set();
    for (const file of files) {
      for (const [, key] of readFileSync(file, "utf8").matchAll(/\bt\(\s*"([A-Za-z][\w.]*)"/g)) {
        if (!(key in en) || !(key in zhCN)) missing.add(`${key} (${relative(root, file)})`);
      }
    }
    expect([...missing]).toEqual([]);
  });

  // HTML 里的 data-i18n 拼错同样不报错，只是界面上留下一串 key。
  // 设置页拆成分页时一次挪了几十个 data-i18n，只能整份 HTML 扫一遍。
  it("HTML 里的 data-i18n key 在两个 locale 里都存在", () => {
    const pages = readdirSync(root).filter((name) => name.endsWith(".html"));
    const missing = new Set();
    for (const page of pages) {
      const html = readFileSync(resolve(root, page), "utf8");
      for (const [, key] of html.matchAll(/data-i18n="([^"]+)"/g)) {
        if (!(key in en) || !(key in zhCN)) missing.add(`${key} (${page})`);
      }
    }
    expect([...missing]).toEqual([]);
  });

  // 窗口速选卡片的状态文案是由 describeWindowProbe 算出来的 key，
  // 不是源码里的 t("…") 字面量，上面那两个扫描都盖不到。
  it("窗口速选服务的每个状态分支都有对应文案", () => {
    const branches = [
      { supported: false },
      { supported: true, active: true },
      { supported: true, active: true, stale: true },
      { supported: true, installed: true, userExtensionsEnabled: false },
      { supported: true, installed: true, userExtensionsEnabled: true },
      { supported: true },
    ];
    const missing = new Set();
    for (const status of branches) {
      const view = describeWindowProbe(status);
      for (const key of [view.stateKey, view.detailKey]) {
        if (!(key in en) || !(key in zhCN)) missing.add(key);
      }
    }
    for (const key of [
      "settings.windowProbe.installedNeedsLogout",
      "settings.windowProbe.installedActive",
      "settings.windowProbe.uninstalled",
      // 覆盖层里的一次性提示，App.tsx 里是三元表达式选出来的 key，扫源码也扫不到
      "capture.windowProbeHint",
      "capture.windowPickingUnavailable",
    ]) {
      if (!(key in en) || !(key in zhCN)) missing.add(key);
    }
    expect([...missing]).toEqual([]);
  });

  it("React 截图和 Pin 文案可随语言切换", () => {
    i18n.init("zh-CN");
    expect(i18n.t("capture.tool.mosaic")).toBe("马赛克");
    expect(i18n.t("capture.translation.localPrivacy")).toContain("本机");
    expect(i18n.t("pin.unlock")).toBe("解锁位置");
    i18n.init("en");
    expect(i18n.t("capture.tool.mosaic")).toBe("Mosaic");
    expect(i18n.t("pin.unlock")).toBe("Unlock position");
  });
});
