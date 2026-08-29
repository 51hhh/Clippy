import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(process.cwd());

function loadEntrypoint(name) {
  return new DOMParser().parseFromString(
    readFileSync(resolve(root, name), "utf8"),
    "text/html",
  );
}

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return /\.(?:js|ts|tsx)$/.test(entry.name) ? [path] : [];
  });
}

describe("built window entrypoints", () => {
  it.each([
    ["index.html", ["app", "clipboard-react-root", "translation-react-root", "preview-panel", "codec-panel"]],
    [
      "settings.html",
      ["theme-grid", "auto-paste-toggle", "translation-group", "capture-commit-action-select"],
    ],
    ["capture.html", ["capture-root"]],
    ["capture-overlay.html", ["root"]],
    ["pin.html", ["root"]],
  ])("contains stable mount points in %s", (name, ids) => {
    const document = loadEntrypoint(name);
    for (const id of ids) {
      expect(document.getElementById(id), `${name}#${id}`).not.toBeNull();
    }
    expect(document.querySelector('script[type="module"]')).not.toBeNull();
  });

  it("keeps the IPC boundary on the typed module", () => {
    const document = loadEntrypoint("index.html");
    const script = document.querySelector('script[type="module"]');
    expect(script?.getAttribute("src")).toBe("js/app.js");
    expect(readFileSync(resolve(root, "js/app.js"), "utf8")).not.toContain("api.js");
  });

  it("does not ship the removed legacy translation panel", () => {
    const document = loadEntrypoint("index.html");
    expect(document.getElementById("translation-panel-legacy")).toBeNull();
    expect(document.getElementById("translation-panel")).toBeNull();
  });

  // 主窗口是无边框悬浮窗，失焦会自动隐藏；原生 <select> 的弹窗在 WebKitGTK 上是
  // 独立 GTK 窗口，一打开就抢走焦点，主窗口随即消失。设置窗口是普通窗口，不受此限。
  it("keeps native <select> out of the auto-hiding main window", () => {
    const document = loadEntrypoint("index.html");
    expect(document.querySelectorAll("select")).toHaveLength(0);

    const codecSelect = document.getElementById("codec-select");
    expect(codecSelect?.classList.contains("custom-select")).toBe(true);
    expect(codecSelect?.querySelector(".custom-select-trigger")).not.toBeNull();
    expect(codecSelect?.querySelectorAll(".custom-select-option").length).toBeGreaterThanOrEqual(22);
    // "最近使用"分组由 codec.js 动态填充，标题与容器必须成对存在
    const recentGroup = document.getElementById("codec-recent-group");
    expect(recentGroup?.classList.contains("custom-select-group")).toBe(true);
    expect(recentGroup?.querySelector(".custom-select-group-title")).not.toBeNull();
    expect(recentGroup?.contains(document.getElementById("codec-recent"))).toBe(true);
  });

  it("keeps direct Tauri access out of production feature modules", () => {
    const apiPath = resolve(root, "js/api.ts");
    const files = [
      ...sourceFiles(resolve(root, "js")),
      ...sourceFiles(resolve(root, "react")),
    ].filter((path) => path !== apiPath);

    for (const path of files) {
      const source = readFileSync(path, "utf8");
      expect(source, path).not.toContain("@tauri-apps/");
      expect(source, path).not.toMatch(/\binvoke\s*\(/);
      expect(source, path).not.toMatch(/\blisten\s*\(/);
    }
  });
});
