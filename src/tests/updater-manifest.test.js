import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import {
  UPDATER_PLATFORMS,
  artifactName,
  buildUpdaterManifest,
  readUpdaterSignatures,
  selectUpdaterPlatforms,
} from "../../scripts/generate-updater-manifest.mjs";

const repoRoot = resolve(process.cwd(), "..");
const temporaryDirectories = [];

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function signatureFixture() {
  return Object.fromEntries(UPDATER_PLATFORMS.map((spec) => [spec.key, `signature:${spec.key}`]));
}

describe("updater 平台契约", () => {
  it("平台键、bundle target 与各自 Tauri 配置一致", () => {
    expect(UPDATER_PLATFORMS.map((spec) => spec.key)).toEqual([
      "linux-x86_64",
      "windows-x86_64",
      "darwin-aarch64",
      "darwin-x86_64",
    ]);

    for (const spec of UPDATER_PLATFORMS) {
      const config = JSON.parse(readFileSync(resolve(repoRoot, spec.config), "utf8"));
      expect(config.bundle.targets, spec.key).toContain(spec.bundleTarget);
    }
  });

  it("生成 Tauri v2 静态 manifest 并内嵌四个平台的签名", () => {
    const version = "1.2.3";
    const signatures = signatureFixture();
    const manifest = buildUpdaterManifest({
      version,
      notes: "修复截图与粘贴\n",
      pubDate: "2026-09-02T01:02:03Z",
      baseUrl: "https://github.com/51hhh/Clippy/releases/download/v1.2.3",
      signatures,
    });

    expect(manifest.version).toBe(version);
    expect(manifest.notes).toBe("修复截图与粘贴");
    expect(Object.keys(manifest.platforms)).toEqual(UPDATER_PLATFORMS.map((spec) => spec.key));
    for (const spec of UPDATER_PLATFORMS) {
      expect(manifest.platforms[spec.key]).toEqual({
        signature: signatures[spec.key],
        url: `https://github.com/51hhh/Clippy/releases/download/v1.2.3/${artifactName(spec, version)}`,
      });
      expect(manifest.platforms[spec.key].signature).not.toMatch(/^https?:/);
    }
  });

  it("从发布目录读取与 artifact 精确同名的签名", () => {
    const version = "1.2.3";
    const directory = mkdtempSync(join(tmpdir(), "clippy-updater-signatures-"));
    temporaryDirectories.push(directory);
    for (const spec of UPDATER_PLATFORMS) {
      writeFileSync(
        join(directory, `${artifactName(spec, version)}.sig`),
        ` signature:${spec.key} \n`,
      );
    }

    expect(readUpdaterSignatures(directory, version)).toEqual(signatureFixture());
  });

  it("未发布 macOS 时只要求并生成 Linux/Windows 平台", () => {
    const platformKeys = ["linux-x86_64", "windows-x86_64"];
    const signatures = Object.fromEntries(
      Object.entries(signatureFixture()).filter(([key]) => platformKeys.includes(key)),
    );
    const manifest = buildUpdaterManifest({
      version: "1.2.3",
      notes: "",
      pubDate: "2026-09-02T01:02:03Z",
      baseUrl: "https://example.test/release",
      signatures,
      platformKeys,
    });
    expect(Object.keys(manifest.platforms)).toEqual(platformKeys);
  });

  it.each([
    ["空列表", [], /至少需要/],
    ["重复平台", ["linux-x86_64", "linux-x86_64"], /不能重复/],
    ["未知平台", ["freebsd-x86_64"], /不支持/],
  ])("拒绝%s", (_name, platformKeys, expectedError) => {
    expect(() => selectUpdaterPlatforms(platformKeys)).toThrow(expectedError);
  });

  it.each([
    ["错误版本", { version: "v1.2.3" }, /SemVer/],
    ["错误时间", { pubDate: "2026-09-02" }, /RFC 3339/],
    ["非 HTTPS", { baseUrl: "http://example.test/release" }, /HTTPS/],
    ["缺少签名", { signatures: { ...signatureFixture(), "darwin-x86_64": "" } }, /signature/],
  ])("拒绝%s", (_name, override, expectedError) => {
    expect(() =>
      buildUpdaterManifest({
        version: "1.2.3",
        notes: "",
        pubDate: "2026-09-02T01:02:03Z",
        baseUrl: "https://example.test/release",
        signatures: signatureFixture(),
        ...override,
      }),
    ).toThrow(expectedError);
  });
});
