// @vitest-environment node
import { readFileSync, readdirSync } from "node:fs";
import { expect, it } from "vitest";

it("grants native dragging only to movable UI without a globally effective Tauri deny", () => {
  const directory = new URL("../../src-tauri/capabilities/", import.meta.url);
  const capabilities = readdirSync(directory).filter(name => name.endsWith(".json"))
    .map(name => JSON.parse(readFileSync(new URL(name, directory), "utf8")));
  // Tauri 2.10.3 的 deny map 命中直接返回 None，不能用窗口限定的 deny 保护覆盖层。
  expect(capabilities.flatMap(capability => capability.permissions)).not.toContain("core:window:deny-start-dragging");
  expect(capabilities.flatMap(capability => capability.permissions)).not.toContain("core:window:deny-start-resize-dragging");
  const grants = capabilities.filter(capability => capability.permissions.includes("core:window:allow-start-dragging"));
  expect(grants).toHaveLength(1);
  expect(grants[0].windows).toEqual(["pin-*", "longshot-controller-*"]);
  expect(grants[0].webviews).toBeUndefined();
});
