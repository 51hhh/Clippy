import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, expect, it, vi } from "vitest";
import { PinToolbar } from "../react/pin/PinToolbar";

let root;
afterEach(async () => {
  if (root) await act(async () => root?.unmount());
  root = undefined;
  document.head.querySelectorAll("[data-pin-height-test]").forEach((node) => node.remove());
  delete globalThis.IS_REACT_ACT_ENVIRONMENT;
});

it("keeps every image Pin control inside the native minimum window height", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const testDirectory = dirname(fileURLToPath(import.meta.url));
  const native = readFileSync(resolve(testDirectory, "../../src-tauri/src/pin/window.rs"), "utf8");
  const height = Number(native.match(/const MIN_OUTER_HEIGHT: f64 = ([\d.]+);/)?.[1]);
  expect(Number.isFinite(height)).toBe(true);
  const sheet = document.createElement("style");
  sheet.dataset.pinHeightTest = "true";
  sheet.textContent = readFileSync(resolve(testDirectory, "../react/pin/pin.css"), "utf8");
  document.head.append(sheet);
  const host = document.createElement("div");
  document.body.replaceChildren(host);
  root = createRoot(host);
  const noop = vi.fn();
  await act(async () => root.render(React.createElement(PinToolbar, {
    media: { x: 12, y: 12, width: 240, height: 120 },
    bounds: { x: 0, y: 0, width: 308, height },
    scale: 1, opacity: 1, locked: false, above: false,
    aboveSupported: true, aboveLimited: false, canvasOpen: false, canSave: true,
    workspaceSaved: false, copied: false, opacityOpen: false,
    onScale: noop, onOpacity: noop, onToggleOpacity: noop,
    onToggleLock: noop, onToggleAbove: noop, onToggleCanvas: noop,
    onCopy: noop, onSave: noop, onWorkspace: noop, onClose: noop,
  })));
  const controls = host.querySelector(".pin-controls");
  const column = host.querySelector(".pin-tools-vertical");
  const layout = getComputedStyle(column);
  const pixels = (value) => Number.parseFloat(value) || 0;
  // jsdom 无布局引擎：依据真实 CSS 和已渲染按钮计算占用，而不是复用定位实现。
  const contentHeight = [...column.children].reduce((sum, child) => {
    const style = getComputedStyle(child);
    return sum + (pixels(style.height) || pixels(style.lineHeight))
      + pixels(style.marginTop) + pixels(style.marginBottom);
  }, 0);
  const toolbarHeight = contentHeight + pixels(layout.gap) * (column.children.length - 1)
    + pixels(layout.paddingTop) + pixels(layout.paddingBottom)
    + pixels(layout.borderTopWidth) + pixels(layout.borderBottomWidth);
  expect(column.querySelectorAll(".pin-tool-button")).toHaveLength(10);
  expect(toolbarHeight).toBeGreaterThan(300);
  expect(Number.parseFloat(controls.style.top)).toBeGreaterThanOrEqual(8);
  expect(Number.parseFloat(controls.style.top) + toolbarHeight).toBeLessThanOrEqual(height - 8);
});
