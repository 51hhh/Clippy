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
      ["theme-grid", "auto-paste-toggle", "translation-group"],
    ],
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
    // "收藏"分组由 codec.js 动态填充，标题与容器必须成对存在
    const favoritesGroup = document.getElementById("codec-favorites-group");
    expect(favoritesGroup?.classList.contains("custom-select-group")).toBe(true);
    expect(favoritesGroup?.querySelector(".custom-select-group-title")).not.toBeNull();
    expect(favoritesGroup?.contains(document.getElementById("codec-favorites"))).toBe(true);
  });

  // 用户反馈过"侧栏按钮和操作名没跟随语言"：静态文案必须挂 data-i18n，否则永远是英文
  it("marks every codec label and tooltip for translation", () => {
    const document = loadEntrypoint("index.html");
    const panel = document.getElementById("codec-panel");
    for (const option of panel.querySelectorAll(".custom-select-option")) {
      expect(option.dataset.i18n, option.dataset.value).toBe(`codec.op.${option.dataset.value}`);
    }
    for (const title of panel.querySelectorAll(".custom-select-group-title")) {
      expect(title.dataset.i18n, title.textContent).toMatch(/^codec\.group\./);
    }
    for (const id of ["codec-swap-dir", "codec-swap", "codec-clear", "codec-copy"]) {
      const button = document.getElementById(id);
      expect(button?.dataset.i18n, id).toMatch(/^codec\.action\./);
      expect(button?.dataset.i18nAttr, id).toBe("title");
    }
    expect(document.getElementById("codec-input")?.dataset.i18nAttr).toBe("placeholder");
    // 收藏按钮的文案随状态在两个键之间切换，由 codec.js 写入
    expect(document.getElementById("codec-favorite")?.dataset.i18n).toBeUndefined();
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
