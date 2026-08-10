import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(process.cwd());

function loadEntrypoint(name) {
  return new DOMParser().parseFromString(
    readFileSync(resolve(root, name), "utf8"),
    "text/html",
  );
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
});
