import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { init } from "../i18n/i18n.js";
import type { ActionDescriptor, ActionLauncherSettings } from "../js/api.ts";
import { LauncherApp } from "../react/launcher/App";
import type { LauncherServices } from "../react/launcher/api";

const platforms = ["linux", "windows", "macos"] as const;
const descriptors: ActionDescriptor[] = [
  { id: "capture.start", input: "unit", output: "capture_session", permissions: ["screen.capture"], cancellable: false, platforms: [...platforms] },
  { id: "image.ocr", input: "owned_image", output: "recognized_text", permissions: ["image.local_analysis"], cancellable: true, platforms: [...platforms] },
  { id: "image.pin", input: "owned_image", output: "window_handle", permissions: ["window.create"], cancellable: false, platforms: [...platforms] },
  { id: "image.save", input: "owned_image", output: "saved_path", permissions: ["file.write"], cancellable: false, platforms: [...platforms] },
  { id: "image.scan_codes", input: "owned_image", output: "detected_codes", permissions: ["image.local_analysis"], cancellable: true, platforms: [...platforms] },
  { id: "text.copy", input: "text", output: "unit", permissions: ["clipboard.write"], cancellable: false, platforms: [...platforms] },
  { id: "text.translate", input: "translation_request", output: "translated_text", permissions: ["translation.network"], cancellable: true, platforms: [...platforms] },
];
const settings: ActionLauncherSettings = {
  theme: "light", language: "en", translationSourceLanguage: "auto", translationTargetLanguage: "en",
};
const deferred = <T,>() => {
  let resolve!: (value: T) => void, reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

let root: Root;
let host: HTMLDivElement;
let services: LauncherServices;
let nativeClose: (() => void) | undefined;

function button(label: string): HTMLButtonElement {
  const node = [...document.querySelectorAll("button")].find(candidate => candidate.textContent?.includes(label));
  expect(node, label).toBeTruthy();
  return node as HTMLButtonElement;
}
async function click(label: string) {
  await act(async () => button(label).click());
}
async function typeInto(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(element), "value")?.set;
  await act(async () => {
    setter?.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
  });
}
async function flush() { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); }

function service(overrides: Partial<LauncherServices> = {}): LauncherServices {
  return {
    discover: vi.fn().mockResolvedValue(descriptors),
    prepare: vi.fn().mockResolvedValue({ requestSlot: "launcher.test", generation: 1 }),
    run: vi.fn().mockResolvedValue({ handle: { requestSlot: "launcher.test", generation: 1 }, output: { type: "unit" } }),
    cancel: vi.fn().mockResolvedValue(undefined),
    settings: vi.fn().mockResolvedValue(settings),
    ready: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
    startDrag: vi.fn().mockResolvedValue(undefined),
    onCloseRequested: vi.fn(async callback => { nativeClose = callback; return () => {}; }),
    ...overrides,
  } as LauncherServices;
}

async function mount() {
  await act(async () => root.render(<LauncherApp settings={settings} services={services} />));
  await flush();
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  init("en");
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
  nativeClose = undefined; services = service();
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks();
});

describe("action launcher", () => {
  it("shows only direct authorized inputs and supports keyboard selection", async () => {
    await mount();
    expect(button("Take screenshot")).toBeTruthy();
    expect(button("Copy text")).toBeTruthy();
    expect(button("Translate text")).toBeTruthy();
    expect(document.body.textContent).not.toContain("image.ocr");
    expect(document.body.textContent).not.toContain("Scan image codes");
    expect(services.ready).toHaveBeenCalledTimes(1);

    const search = document.querySelector<HTMLInputElement>('.launcher-search input')!;
    expect(document.activeElement).toBe(search);
    await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
    await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(document.querySelector(".launcher-detail h1")?.textContent).toBe("Copy text");
  });

  it("renders a useful empty state and restores results when the query changes", async () => {
    await mount();
    const search = document.querySelector<HTMLInputElement>('.launcher-search input')!;
    await typeInto(search, "does-not-exist");
    expect(document.querySelector(".launcher-empty")?.textContent).toContain("No matching actions");
    await typeInto(search, "translate");
    expect(document.querySelectorAll(".launcher-action")).toHaveLength(1);
    expect(document.querySelector(".launcher-action")?.textContent).toContain("Translate text");
  });

  it("keeps a cancellable action in running state until its exact handle is cancelled", async () => {
    const pending = deferred<never>();
    const handle = { requestSlot: "launcher.translate", generation: 7 };
    services = service({
      prepare: vi.fn().mockResolvedValue(handle),
      run: vi.fn(() => pending.promise),
    });
    await mount(); await click("Translate text");
    await typeInto(document.querySelector("textarea")!, "hello");
    await click("Run action"); await flush();
    expect(document.querySelector(".launcher-running")?.textContent).toContain("Running action");
    await click("Cancel");
    expect(services.cancel).toHaveBeenCalledWith(handle);
    await act(async () => pending.reject({ code: "action_cancelled" }));
    expect(document.querySelector("[role=status]")?.textContent).toContain("Action cancelled");
    expect(document.querySelector(".launcher-form")).toBeTruthy();
  });

  it("maps stable errors without displaying backend details", async () => {
    services = service({ run: vi.fn().mockRejectedValue({ code: "action_clipboard_failed", detail: "/private/path" }) });
    await mount(); await click("Copy text");
    await typeInto(document.querySelector("textarea")!, "hello");
    await click("Run action"); await flush();
    expect(document.querySelector("[role=alert]")?.textContent).toContain("system clipboard could not be updated");
    expect(document.body.textContent).not.toContain("/private/path");
  });

  it("native close cancels a running action before destroying the window", async () => {
    const pending = deferred<never>();
    const handle = { requestSlot: "launcher.translate", generation: 11 };
    services = service({ prepare: vi.fn().mockResolvedValue(handle), run: vi.fn(() => pending.promise) });
    await mount(); await click("Translate text");
    await typeInto(document.querySelector("textarea")!, "hello"); await click("Run action"); await flush();
    await act(async () => { nativeClose?.(); await Promise.resolve(); await Promise.resolve(); });
    expect(services.cancel).toHaveBeenCalledWith(handle);
    expect(services.close).toHaveBeenCalledTimes(1);
  });

  it("keeps the launcher usable when native close-listener setup throws synchronously", async () => {
    services = service({ onCloseRequested: (() => { throw new Error("unavailable"); }) as LauncherServices["onCloseRequested"] });
    await mount();
    expect(button("Take screenshot")).toBeTruthy();
    expect(document.querySelector("[role=alert]")?.textContent).toContain("actions window could not be updated");
  });
});
