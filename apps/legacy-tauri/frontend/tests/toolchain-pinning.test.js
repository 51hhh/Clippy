// @vitest-environment node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const repository = resolve(process.cwd(), "../../..");
const read = (path) => readFileSync(resolve(repository, path), "utf8").replace(/\r\n/g, "\n");

describe("Rust 工具链在本地与 CI 保持一致", () => {
  const toolchain = read("rust-toolchain.toml");
  const workflows = [
    read(".github/workflows/build.yml"),
    read(".github/workflows/native-qa.yml"),
    read(".github/workflows/release.yml"),
  ];

  it("固定到经过本地完整门禁验证的版本", () => {
    expect(toolchain).toContain('channel = "1.98.0"');
    expect(toolchain).toContain('components = ["clippy", "rustfmt"]');
  });

  it("每个 GitHub Rust 安装步骤显式使用同一版本", () => {
    for (const workflow of workflows) {
      const setupCount = workflow.match(/uses: dtolnay\/rust-toolchain@/g)?.length ?? 0;
      const pinnedCount = workflow.match(/toolchain: 1\.98\.0/g)?.length ?? 0;
      expect(pinnedCount).toBe(setupCount);
    }
  });
});
