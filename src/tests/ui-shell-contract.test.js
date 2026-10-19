import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(process.cwd());
const read = (path) => readFileSync(resolve(root, path), "utf8");
const THEME_IDS = ["light", "dark", "nord", "solarized-light", "rose", "midnight"];

function themeTokens(css, id) {
  const body = css.match(new RegExp(`\\[data-theme="${id}"\\]\\s*\\{([^}]+)\\}`))?.[1];
  expect(body, `${id} theme block`).toBeTruthy();
  return Object.fromEntries(
    [...body.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})\s*;/gi)].map((match) => [match[1], match[2]]),
  );
}

function rgb(hex) {
  return [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16));
}

function mix(foreground, background, weight) {
  return foreground.map((channel, index) => Math.round(channel * weight + background[index] * (1 - weight)));
}

function luminance(color) {
  const [red, green, blue] = color
    .map((channel) => channel / 255)
    .map((channel) => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrast(first, second) {
  const [bright, dark] = [luminance(first), luminance(second)].sort((left, right) => right - left);
  return (bright + 0.05) / (dark + 0.05);
}

describe("shared window shell contract", () => {
  it("defines every selectable theme", () => {
    const themes = read("styles/themes.css");
    for (const id of THEME_IDS) {
      expect(themes, id).toContain(`[data-theme="${id}"]`);
    }
  });

  it("keeps small success and warning badges readable in every theme", () => {
    const themes = read("styles/themes.css");
    for (const id of THEME_IDS) {
      const tokens = themeTokens(themes, id);
      const background = rgb(tokens["bg-secondary"]);
      for (const [name, weight] of [["success", 0.15], ["warning", 0.16]]) {
        const foreground = rgb(tokens[name]);
        expect(contrast(foreground, mix(foreground, background, weight)), `${id} ${name}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it.each([
    "js/settings.js",
    "react/launcher/App.tsx",
    "react/viewer/App.tsx",
    "react/recording-library/main.tsx",
    "react/pin-workspace-library/main.tsx",
  ])("loads the shared shell in %s", (path) => {
    expect(read(path)).toContain("styles/window-shell.css");
  });

  it.each([
    "react/recording-library/recording-library.css",
    "react/pin-workspace-library/pin-workspace-library.css",
  ])("derives library colors from tokens in %s", (path) => {
    const css = read(path);
    expect(css).not.toContain("data-theme=");
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    expect(css).toContain("var(--bg-");
    expect(css).toContain("var(--text-");
    expect(css).toContain("var(--border");
    expect(css).toContain("var(--control-");
  });

  it("creates settings without native decorations and preserves guarded close", () => {
    const windowController = read("../src-tauri/src/window_controller.rs");
    const settingsApi = read("js/api/settings.ts");
    const settings = read("js/settings.js");
    expect(windowController).toMatch(/"settings"[\s\S]*?\.decorations\(false\)[\s\S]*?\.build\(\)/);
    expect(settingsApi).toContain('invoke<void>("close_settings")');
    expect(settings).toContain("requestClose: requestSettingsClose");
  });
});
