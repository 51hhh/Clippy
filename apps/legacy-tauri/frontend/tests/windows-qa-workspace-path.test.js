// @vitest-environment node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const repository = resolve(process.cwd(), "../../..");

describe("Windows QA 运行库只写入 workspace target", () => {
  const prepareRuntime = readFileSync(
    resolve(repository, "scripts/prepare-windows-qa-runtime.ps1"),
    "utf8",
  ).replace(/\r\n/g, "\n");

  it("使用根 workspace 的统一产物目录", () => {
    expect(prepareRuntime).toContain("Join-Path $taskRepository 'target'");
    expect(prepareRuntime).toContain("Join-Path $taskRepository 'target\\windows-qa-runtime'");
    expect(prepareRuntime).not.toContain("apps/legacy-tauri/native\\target");
  });
});
