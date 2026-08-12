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
    ["index.html", ["app", "clip-list", "preview-panel", "codec-panel"]],
    ["settings.html", ["theme-grid", "auto-paste-toggle", "translation-group"]],
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
