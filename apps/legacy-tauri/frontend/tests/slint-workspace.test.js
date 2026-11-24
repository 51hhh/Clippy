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
    expect(workspace).toContain('"crates/clippy-platform"');
    const defaults = workspace.match(/default-members = \[([\s\S]*?)\]/)?.[1] ?? "";
    expect(defaults).toContain('"apps/slint"');
    expect(defaults).not.toContain('"apps/legacy-tauri/native"');
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
    const platformManifest = read("crates/clippy-platform/Cargo.toml");
    const legacyManifest = read("apps/legacy-tauri/native/Cargo.toml");
    const slintManifest = read("apps/slint/Cargo.toml");
    expect(coreManifest).not.toMatch(/^tauri\s*=/m);
    expect(coreManifest).not.toMatch(/^slint\s*=/m);
    expect(platformManifest).not.toMatch(/^tauri\s*=/m);
    expect(platformManifest).not.toMatch(/^slint\s*=/m);
    expect(legacyManifest).toContain('clippy-core = { path = "../../../crates/clippy-core" }');
    expect(slintManifest).toContain('clippy-core = { path = "../../crates/clippy-core" }');
    expect(slintManifest).toContain('clippy-platform = { path = "../../crates/clippy-platform" }');
  });

  it("checks and tests shared core on local and three-platform CI gates", () => {
    const localGate = read("scripts/ci-local.sh");
    const buildWorkflow = read(".github/workflows/build.yml");
    const windowsGate = read("scripts/ci-windows.ps1");
    for (const source of [localGate, buildWorkflow]) {
      expect(source).toContain("cargo check --locked -p clippy-core -p clippy-platform --all-targets");
      expect(source).toContain("cargo test --locked -p clippy-core -p clippy-platform");
    }
    expect(windowsGate).toContain("cargo.exe check --locked -p clippy-core -p clippy-platform --all-targets");
    expect(windowsGate).toContain("cargo.exe test --locked -p clippy-core -p clippy-platform");
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
      expect(source).toContain("cargo test --locked -p clippy-slint --features qa-build");
    }
    expect(windowsGate).toContain("cargo.exe check --locked -p clippy-slint --all-targets");
    expect(windowsGate).toContain("cargo.exe test --locked -p clippy-slint");
    expect(windowsGate).toContain("cargo.exe test --locked -p clippy-slint --features qa-build");
    expect(buildWorkflow.match(/name: Slint Rust check/g)).toHaveLength(2);
    expect(buildWorkflow.match(/name: Slint Rust tests/g)).toHaveLength(2);
    expect(buildWorkflow.match(/name: Slint QA profile tests/g)).toHaveLength(2);
  });

  it("keeps the Slint QA identity separate before platform services are connected", () => {
    const identity = read("crates/clippy-platform/src/identity.rs");
    const slintManifest = read("apps/slint/Cargo.toml");
    const productionMain = read("apps/slint/src/bin/clippy-slint.rs");
    const qaMain = read("apps/slint/src/bin/clippy-slint-qa.rs");
    const qaPackager = read("apps/slint/Packager.qa.toml");
    expect(identity).toContain('application_id: "com.clippy.qa.slint"');
    expect(identity).toContain('data_namespace: "com.clippy.qa.slint"');
    expect(identity).toContain('shortcut_namespace: "com.clippy.qa.slint.shortcuts"');
    expect(slintManifest).toMatch(/^qa-build = \[\]$/m);
    expect(slintManifest).toContain('name = "clippy-slint-qa"');
    expect(slintManifest).toContain('required-features = ["qa-build"]');
    expect(productionMain).toContain("PRODUCTION_IDENTITY");
    expect(productionMain).not.toContain("SLINT_QA_IDENTITY");
    expect(qaMain).toContain("SLINT_QA_IDENTITY");
    expect(qaMain).not.toContain("PRODUCTION_IDENTITY");
    expect(qaPackager).toMatch(/^identifier = "com\.clippy\.qa\.slint"$/m);
    expect(qaPackager).toMatch(/^authors = \["zhongweixi2000"\]$/m);
    expect(qaPackager).toContain('binaries = [{ path = "clippy-slint-qa", main = true }]');
    expect(qaPackager).toContain('depends = ["libc6", "libfontconfig1", "libgcc-s1"]');
    expect(qaPackager).toContain('minimum-system-version = "12.3"');
    expect(qaPackager).toContain('signing-identity = "-"');
    expect(qaPackager).toContain("LicenseRef-Slint-Royalty-free-2.0.md");
    expect(qaPackager).not.toContain("before-packaging-command");
  });

  it("builds isolated legacy and Slint QA artifacts on all native platforms", () => {
    const nativeQa = read(".github/workflows/native-qa.yml");
    expect(nativeQa).toMatch(/default: slint/);
    expect(nativeQa.match(/if: inputs\.backend == 'slint' \|\| inputs\.backend == 'all'/g)).toHaveLength(3);
    expect(nativeQa.match(/if: inputs\.backend == 'legacy' \|\| inputs\.backend == 'all'/g)).toHaveLength(4);
    expect(nativeQa).toContain("build-slint-linux:");
    expect(nativeQa).toContain("build-slint-windows:");
    expect(nativeQa).toContain("build-slint-macos:");
    expect(nativeQa.match(/cargo install cargo-packager --version 0\.11\.8 --locked/g)).toHaveLength(3);
    expect(nativeQa.match(/cargo build --locked --release -p clippy-slint/g)).toHaveLength(3);
    expect(nativeQa).toContain("--formats deb,appimage");
    expect(nativeQa).toContain("--formats nsis,wix");
    expect(nativeQa).toContain("--formats app,dmg");
    expect(nativeQa.match(/--backend legacy-tauri/g)).toHaveLength(3);
    expect(nativeQa.match(/--backend slint/g)).toHaveLength(3);
    for (const artifact of [
      "qa-legacy-linux-x64-",
      "qa-legacy-windows-x64-",
      "qa-legacy-macos-",
      "qa-slint-linux-x64-",
      "qa-slint-windows-x64-",
      "qa-slint-macos-",
    ]) {
      expect(nativeQa).toContain(artifact);
    }
  });

  it("uses core plus Slint as the default gate and keeps legacy explicit", () => {
    const localGate = read("scripts/ci-local.sh");
    const windowsGate = read("scripts/ci-windows.ps1");
    expect(localGate).toContain("--segment current|all|core|legacy|slint");
    expect(localGate).toContain("SEGMENT=current");
    expect(localGate).toContain("run_core_segment");
    expect(localGate).toContain("run_legacy_segment");
    expect(localGate).toContain("run_slint_segment");
    expect(localGate).toContain("证据级别：部分证据");
    expect(windowsGate).toContain("[ValidateSet('Current', 'All', 'Core', 'Legacy', 'Slint')]");
    expect(windowsGate).toContain("[string]$Segment = 'Current'");
    expect(windowsGate).toContain("Evidence level: partial");
  });

  it("does not compile legacy in the active v0.2 workflow", () => {
    const buildWorkflow = read(".github/workflows/build.yml");
    const activeJobs = buildWorkflow.split("  recording-codec-prototype:")[0];
    expect(activeJobs).not.toContain("apps/legacy-tauri");
    expect(activeJobs).not.toContain("libwebkit2gtk");
    expect(buildWorkflow).toContain("inputs.legacy_recording_prototype");
  });

  it("audits the root lockfile shared by both applications", () => {
    const buildWorkflow = read(".github/workflows/build.yml");
    const attributes = read(".gitattributes");
    expect(buildWorkflow).toContain("target/cargo-audit/bin/cargo-audit audit");
    expect(attributes).toMatch(/^\/Cargo\.lock text eol=lf$/m);
    expect(attributes).not.toContain("apps/legacy-tauri/native/Cargo.lock");
  });
});
