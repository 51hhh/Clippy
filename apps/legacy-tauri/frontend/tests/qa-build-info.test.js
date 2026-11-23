// @vitest-environment node
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { buildQaBuildInfo, parseArguments } from "../../../../scripts/qa-build-info.mjs";

const repositoryRoot = resolve(process.cwd(), "../../..");
const sha = "0123456789abcdef0123456789abcdef01234567";

describe("QA build metadata", () => {
  it("describes legacy without claiming Slint is included", () => {
    const result = buildQaBuildInfo({
      backend: "legacy-tauri",
      platform: "linux-x64",
      signing: "unsigned-qa-only",
      sha,
      fields: ["recording_feature=recording-wayland-qa"],
    }, repositoryRoot);
    expect(result).toContain("version=0.1.21\n");
    expect(result).toContain("backend=legacy-tauri\n");
    expect(result).toContain("framework=Tauri\nframework_version=2.12.4\n");
    expect(result).toContain("renderer=webview\nslint=not included\n");
    expect(result).not.toContain("identity=");
  });

  it("describes Slint from the checked-in build configuration", () => {
    const result = buildQaBuildInfo({
      backend: "slint",
      platform: "windows-x64",
      signing: "unsigned-qa-only",
      sha: sha.toUpperCase(),
      fields: ["packager=cargo-packager-0.11.8"],
    }, repositoryRoot);
    expect(result).toContain(`commit=${sha}\nversion=0.2.0\n`);
    expect(result).toContain("backend=slint\nframework=Slint\nframework_version=1.19.0\n");
    expect(result).toContain("renderer=femtovg\nidentity=com.clippy.qa.slint\n");
    expect(result).toContain("data_namespace=com.clippy.qa.slint\n");
    expect(result).toContain("single_instance_key=com.clippy.qa.slint\n");
    expect(result).toContain("shortcut_namespace=com.clippy.qa.slint.shortcuts\n");
    expect(result).toContain("tauri=not included\npackager=cargo-packager-0.11.8\n");
  });

  it("rejects malformed commits and reserved field overrides", () => {
    expect(() => buildQaBuildInfo({
      backend: "slint",
      platform: "linux-x64",
      signing: "unsigned",
      sha: "short",
    }, repositoryRoot)).toThrow(/40 位/);
    expect(() => buildQaBuildInfo({
      backend: "slint",
      platform: "linux-x64",
      signing: "unsigned",
      sha,
      fields: ["renderer=skia"],
    }, repositoryRoot)).toThrow(/保留字段/);
  });

  it("parses repeated extension fields without accepting unknown options", () => {
    expect(parseArguments([
      "--backend", "slint",
      "--platform", "linux-x64",
      "--signing", "unsigned",
      "--sha", sha,
      "--output", "QA-BUILD.txt",
      "--field", "one=1",
      "--field", "two=2",
    ]).fields).toEqual(["one=1", "two=2"]);
    expect(() => parseArguments([
      "--backend", "slint",
      "--platform", "linux-x64",
      "--signing", "unsigned",
      "--sha", sha,
      "--output", "QA-BUILD.txt",
      "--unexpected", "value",
    ])).toThrow(/未知参数/);
  });
});
