// @vitest-environment node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const repository = resolve(process.cwd(), "../../..");
const workflow = readFileSync(resolve(repository, ".github/workflows/build.yml"), "utf8")
  .replace(/\r\n/g, "\n");

describe("Rust 依赖审计使用进程退出码作为门禁", () => {
  it("固定 cargo-audit 版本且不依赖 GitHub Checks API", () => {
    expect(workflow).toContain(
      "cargo install cargo-audit --version 0.22.2 --locked --root target/cargo-audit",
    );
    expect(workflow).toContain("target/cargo-audit/bin/cargo-audit audit");
    expect(workflow).not.toContain("rustsec/audit-check@");
    expect(workflow).not.toContain("checks: write");
  });
});
