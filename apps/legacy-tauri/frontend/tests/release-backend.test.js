import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import {
  formatGitHubOutput,
  parseReleaseTag,
  resolveReleaseBackend,
} from "../../../../scripts/resolve-release-backend.mjs";

describe("release tag 后端路由", () => {
  it.each([
    ["v0.1.21", "legacy-tauri", "Tauri", "2.12.4"],
    ["v0.1.99", "legacy-tauri", "Tauri", "2.12.4"],
    ["v0.2.0-alpha.1", "slint", "Slint", "1.19.0"],
    ["v0.2.0-beta.2", "slint", "Slint", "1.19.0"],
    ["v0.2.0-rc.1", "slint", "Slint", "1.19.0"],
    ["v0.2.0", "slint", "Slint", "1.19.0"],
    ["v0.10.0", "slint", "Slint", "1.19.0"],
    ["v1.0.0", "slint", "Slint", "1.19.0"],
  ])("%s 只选择预期实现", (tag, backend, framework, frameworkVersion) => {
    expect(resolveReleaseBackend(tag)).toMatchObject({
      tag,
      version: tag.slice(1),
      backend,
      framework,
      frameworkVersion,
    });
  });

  it.each([
    "0.2.0",
    "v0.2",
    "v0.2.0-",
    "v0.2.0-alpha..1",
    "v0.2.0-01",
    "v00.2.0",
    "release-v0.2.0",
  ])("拒绝非法 tag %s", (tag) => {
    expect(() => parseReleaseTag(tag)).toThrow();
  });

  it("把预发布与构建元数据原样传给发布记录", () => {
    expect(resolveReleaseBackend("v0.2.0-rc.1+qa.7")).toMatchObject({
      prerelease: "rc.1",
      build: "qa.7",
      backend: "slint",
    });
  });

  it("生成单一来源的 GitHub Actions outputs", () => {
    const output = formatGitHubOutput(resolveReleaseBackend("v0.2.0"));

    expect(output.split("\n")).toEqual([
      "version=0.2.0",
      "backend=slint",
      "framework=Slint",
      "framework_version=1.19.0",
      "excluded_framework=Tauri",
    ]);
  });

  it("release workflow 只从受测解析器取得版本与后端", () => {
    const workflow = readFileSync(resolve("../../../.github/workflows/release.yml"), "utf8");

    expect(workflow).toContain("node scripts/resolve-release-backend.mjs");
    expect(workflow).toContain("backend: ${{ steps.release_route.outputs.backend }}");
    expect(workflow).toContain("framework_version: ${{ steps.release_route.outputs.framework_version }}");
    expect(workflow).not.toContain('TAG_VERSION="${GITHUB_REF#refs/tags/v}"');
    expect(workflow).toContain("Slint tag routing is active, but the v0.2 packaging job is not enabled yet");
  });
});
