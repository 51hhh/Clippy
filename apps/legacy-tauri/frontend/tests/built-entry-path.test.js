// @vitest-environment node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const repository = resolve(process.cwd(), "../../..");
const read = (path) => readFileSync(resolve(repository, path), "utf8").replace(/\r\n/g, "\n");

describe("生产入口检查不依赖调用目录", () => {
  const checker = read("scripts/check-built-main.mjs");
  const vite = read("apps/legacy-tauri/frontend/vite.config.mjs");

  it("默认读取 Vite 配置声明的 legacy 产物目录", () => {
    expect(vite).toContain('outDir: "../dist"');
    expect(checker).toContain('resolve(repository, "apps/legacy-tauri/dist")');
    expect(checker).not.toContain('resolve(repository, "dist")');
  });
});
