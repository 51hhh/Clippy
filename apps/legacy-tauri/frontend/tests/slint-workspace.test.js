// @vitest-environment node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const repositoryRoot = resolve(process.cwd(), "../../..");

function read(path) {
  return readFileSync(resolve(repositoryRoot, path), "utf8");
}

describe("Slint v0.2 migration workspace contract", () => {
  it("keeps legacy, Slint and shared core in one workspace", () => {
    const workspace = read("Cargo.toml");
    expect(workspace).toContain('"apps/legacy-tauri/native"');
    expect(workspace).toContain('"apps/slint"');
    expect(workspace).toContain('"crates/clippy-core"');
  });

  it("pins the new application to v0.2.0 and Slint 1.19.0", () => {
    const manifest = read("apps/slint/Cargo.toml");
    const legacyManifest = read("apps/legacy-tauri/native/Cargo.toml");
    expect(manifest).toMatch(/^version = "0\.2\.0"$/m);
    expect(manifest).toMatch(/^slint = \{ version = "=1\.19\.0"/m);
    expect(manifest).toMatch(/^slint-build = \{ version = "=1\.19\.0"/m);
    expect(manifest).not.toMatch(/^tauri\s*=/m);
    expect(legacyManifest).toMatch(/^objc2 = \{ version = "=0\.6\.5"/m);
  });

  it("shares a UI-independent core between both application hosts", () => {
    const coreManifest = read("crates/clippy-core/Cargo.toml");
    const legacyManifest = read("apps/legacy-tauri/native/Cargo.toml");
    const slintManifest = read("apps/slint/Cargo.toml");
    expect(coreManifest).not.toMatch(/^tauri\s*=/m);
    expect(coreManifest).not.toMatch(/^slint\s*=/m);
    expect(legacyManifest).toContain('clippy-core = { path = "../../../crates/clippy-core" }');
    expect(slintManifest).toContain('clippy-core = { path = "../../crates/clippy-core" }');
  });

  it("checks and tests shared core on local and three-platform CI gates", () => {
    const localGate = read("scripts/ci-local.sh");
    const buildWorkflow = read(".github/workflows/build.yml");
    const windowsGate = read("scripts/ci-windows.ps1");
    for (const source of [localGate, buildWorkflow]) {
      expect(source).toContain("cargo check --locked -p clippy-core --all-targets");
      expect(source).toContain("cargo test --locked -p clippy-core");
    }
    expect(windowsGate).toContain("cargo.exe check --locked -p clippy-core --all-targets");
    expect(windowsGate).toContain("cargo.exe test --locked -p clippy-core");
    expect(buildWorkflow.match(/name: Core Rust check/g)).toHaveLength(2);
    expect(buildWorkflow.match(/name: Core Rust tests/g)).toHaveLength(2);
  });

  it("checks and tests Slint on local and three-platform CI gates", () => {
    const localGate = read("scripts/ci-local.sh");
    const buildWorkflow = read(".github/workflows/build.yml");
    const windowsGate = read("scripts/ci-windows.ps1");
    for (const source of [localGate, buildWorkflow]) {
      expect(source).toContain("cargo check --locked -p clippy-slint --all-targets");
      expect(source).toContain("cargo test --locked -p clippy-slint");
    }
    expect(windowsGate).toContain("cargo.exe check --locked -p clippy-slint --all-targets");
    expect(windowsGate).toContain("cargo.exe test --locked -p clippy-slint");
    expect(buildWorkflow.match(/name: Slint Rust check/g)).toHaveLength(2);
    expect(buildWorkflow.match(/name: Slint Rust tests/g)).toHaveLength(2);
  });

  it("audits the root lockfile shared by both applications", () => {
    const buildWorkflow = read(".github/workflows/build.yml");
    const attributes = read(".gitattributes");
    expect(buildWorkflow).toMatch(
      /name: Rust dependency audit[\s\S]*?working-directory: \.[\s\S]*?name: X11 clipboard/,
    );
    expect(attributes).toMatch(/^\/Cargo\.lock text eol=lf$/m);
    expect(attributes).not.toContain("apps/legacy-tauri/native/Cargo.lock");
  });
});
