#!/usr/bin/env node
// 导入实际生产主入口；仅替换Tauri传输，不重编译源码或绕过无参数初始化路径。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(resolve(repository, "src/package.json"));
const { JSDOM } = require("jsdom");
const output = resolve(process.argv[2] || resolve(repository, "dist"));
const dom = new JSDOM(readFileSync(resolve(output, "index.html"), "utf8"), {
  url: "https://clippy-build.test/", pretendToBeVisual: true,
});
const { window } = dom;
for (const key of ["window", "document", "navigator", "MutationObserver", "HTMLElement", "Element", "Node", "CustomEvent", "Image", "DOMParser", "getComputedStyle", "requestAnimationFrame", "cancelAnimationFrame"]) {
  Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? window : window[key] });
}
window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
const calls = [];
const idle = { status: "idle", revision: 0, version: null, body: null, install_type: "appimage", downloaded: 0, total: null };
const config = { theme: "light", language: "en", translation_source_language: "auto", translation_target_language: "en", translation_services: [], ocr_enabled: false };
let callbackId = 0;
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  transformCallback: () => ++callbackId, unregisterCallback() {},
  convertFileSrc: () => "about:blank",
  invoke: async (command, args) => {
    calls.push({ command, args });
    if (command === "get_config") return config;
    if (command === "get_clips") return [];
    if (command === "get_app_update_state" || command === "check_app_update") return idle;
    if (command === "plugin:event|listen") return callbackId;
    if (command === "plugin:event|unlisten" || command === "set_preview_visible") return null;
    throw new Error(`Unexpected real-entry IPC in isolated build check: ${command}`);
  },
};
const rejected = [];
const rejection = reason => rejected.push(reason);
process.on("unhandledRejection", rejection);
try {
  const entry = window.document.querySelector('script[type="module"][src]')?.getAttribute("src");
  assert(entry, "built index.html has no module entrypoint");
  await import(pathToFileURL(resolve(output, entry.replace(/^\//, ""))).href);
  // 主入口自身按readyState选择注册/立即执行，不手工调用其内部函数。
  const started = Date.now();
  while (!window.document.getElementById("update-modal")?.hasAttribute("role") && Date.now() - started < 2000 && rejected.length === 0) {
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.deepEqual(rejected.map(String), [], "built entry initialization rejected");
  const modal = window.document.getElementById("update-modal");
  assert.equal(modal?.getAttribute("role"), "dialog", "default update modal did not initialize");
  window.document.getElementById("list-panel").focus();
  const tab = new window.KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
  window.document.activeElement.dispatchEvent(tab);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(tab.defaultPrevented, true, "production keyboard router did not claim list Tab");
  assert(calls.some(call => call.command === "set_preview_visible" && call.args?.visible === true), "list Tab did not open preview through production IPC wrapper");
  assert(calls.some(call => call.command === "check_app_update"), "built entry did not start its automatic update check");
  assert(!calls.some(call => call.command.includes("install") || call.command === "select_clip"), "build smoke caused an output action");
  console.log(JSON.stringify({ result: "passed", entry, updateRole: modal.getAttribute("role"), tabPrevented: tab.defaultPrevented, commands: calls.map(call => call.command) }));
} finally {
  process.removeListener("unhandledRejection", rejection);
  window.close();
}
