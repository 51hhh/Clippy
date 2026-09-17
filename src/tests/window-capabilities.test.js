// @vitest-environment node
import { readFileSync, readdirSync } from "node:fs";
import { expect, it } from "vitest";

function capabilities() {
  const directory = new URL("../../src-tauri/capabilities/", import.meta.url);
  return readdirSync(directory).filter(name => name.endsWith(".json"))
    .map(name => JSON.parse(readFileSync(new URL(name, directory), "utf8")));
}

function permissionIdentifier(permission) {
  return typeof permission === "string" ? permission : permission.identifier;
}

function permissionIdentifiers(capability) {
  return capability.permissions.map(permissionIdentifier);
}

it("grants native dragging only to movable UI without a globally effective Tauri deny", () => {
  const definitions = capabilities();
  // Tauri 2.10.3 的 deny map 命中直接返回 None，不能用窗口限定的 deny 保护覆盖层。
  const identifiers = definitions.flatMap(permissionIdentifiers);
  expect(identifiers).not.toContain("core:window:deny-start-dragging");
  expect(identifiers).not.toContain("core:window:deny-start-resize-dragging");
  const grants = definitions.filter(capability => permissionIdentifiers(capability).includes("core:window:allow-start-dragging"));
  expect(grants).toHaveLength(1);
  expect(grants[0].windows).toEqual(["pin-*", "longshot-controller-*"]);
  expect(grants[0].webviews).toBeUndefined();
});

it("keeps every production window class inside an explicit native capability", () => {
  const definitions = capabilities();
  const expected = new Map([
    ["default", ["main"]],
    ["settings", ["settings"]],
    ["pin", ["pin-*"]],
    ["capture-overlay", ["capture-overlay-*"]],
    ["longshot-controller", ["longshot-controller-*"]],
    ["image-viewer", ["image-viewer-*"]],
  ]);

  for (const [identifier, windows] of expected) {
    expect(definitions.find(capability => capability.identifier === identifier)?.windows).toEqual(windows);
  }
  expect(new Set([...expected.values()].flat())).toEqual(new Set([
    "main",
    "settings",
    "pin-*",
    "capture-overlay-*",
    "longshot-controller-*",
    "image-viewer-*",
  ]));
});

it("grants sensitive native APIs only to the window that uses them", () => {
  const definitions = capabilities();
  const grants = (identifier) => definitions
    .filter(capability => permissionIdentifiers(capability).includes(identifier))
    .map(capability => capability.identifier);

  expect(grants("core:window:allow-hide")).toEqual(["default"]);
  expect(grants("core:window:allow-close")).toEqual(["capture-overlay"]);
  expect(grants("core:app:allow-version")).toEqual(["settings"]);
  expect(grants("autostart:default")).toEqual(["settings"]);
  expect(grants("opener:allow-open-url")).toEqual(["default", "settings"]);

  const openerPermissions = definitions.flatMap(capability => capability.permissions)
    .filter(permission => permissionIdentifier(permission) === "opener:allow-open-url");
  expect(openerPermissions).toHaveLength(2);
  for (const permission of openerPermissions) {
    expect(permission).toEqual({
      identifier: "opener:allow-open-url",
      allow: [{ url: "https://github.com/51hhh/Clippy/*" }],
    });
  }
});

it("does not restore broad frontend-native permission sets", () => {
  const identifiers = capabilities().flatMap(permissionIdentifiers);
  expect(identifiers).not.toContain("core:default");
  expect(identifiers).not.toContain("global-shortcut:default");
  expect(identifiers).not.toContain("opener:default");
  for (const identifier of [
    "core:window:allow-show",
    "core:window:allow-set-focus",
    "core:window:allow-set-size",
    "core:window:allow-set-position",
    "core:window:allow-outer-position",
    "core:window:allow-set-always-on-top",
    "core:window:allow-destroy",
    "core:window:allow-scale-factor",
  ]) {
    expect(identifiers).not.toContain(identifier);
  }
});

it("routes every custom command through the unified business-command gate", () => {
  const lib = readFileSync(new URL("../../src-tauri/src/lib.rs", import.meta.url), "utf8");
  const access = readFileSync(new URL("../../src-tauri/src/ipc_access.rs", import.meta.url), "utf8");

  expect(lib).toContain(".invoke_handler(ipc_access::restrict(tauri::generate_handler![");
  expect(lib).not.toContain("viewer::access::restrict");
  for (const label of [
    '"settings"',
    '"pin-"',
    '"capture-overlay-"',
    '"longshot-controller-"',
    '"image-viewer-"',
  ]) {
    expect(access).toContain(label);
  }
  expect(access).toContain("CallerKind::Unknown => return false");
});
