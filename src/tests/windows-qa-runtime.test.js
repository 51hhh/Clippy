// @vitest-environment node
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { inspectPe, sha256, verifyLocalRuntimeClosure, verifyRuntime } from "../../scripts/verify-windows-qa-runtime.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const source = "a".repeat(40);
const publisher = { status: "Valid", subject: "CN=Microsoft publisher, O=Microsoft Corporation, C=US", thumbprint: "b".repeat(40) };
const pwsh = join(process.env.ProgramFiles || "C:\\Program Files", "PowerShell/7/pwsh.exe");
const shells = ["powershell.exe", ...(existsSync(pwsh) ? [pwsh] : [])];

// 小型 PE 文件夹具只模拟 import 元数据，不是可执行代码，也不加载 DLL。
function pe(imports = [], delay = [], { machine = 0x8664, legacyDelay = false } = {}) {
  const bytes = Buffer.alloc(4096), header = 0x80, opt = header + 24, raw = 0x200, rva = 0x1000;
  bytes.write("MZ"); bytes.writeUInt32LE(header, 0x3c); bytes.write("PE\0\0", header, "binary");
  bytes.writeUInt16LE(machine, header + 4); bytes.writeUInt16LE(1, header + 6); bytes.writeUInt16LE(240, header + 20);
  bytes.writeUInt16LE(0x20b, opt); bytes.writeBigUInt64LE(legacyDelay ? 0x400000n : 0x140000000n, opt + 24);
  bytes.writeUInt32LE(raw, opt + 60); bytes.writeUInt32LE(16, opt + 108);
  bytes.writeUInt32LE(rva, opt + 240 + 12); bytes.writeUInt32LE(bytes.length - raw, opt + 240 + 16); bytes.writeUInt32LE(raw, opt + 240 + 20);
  let next = raw + 0x500;
  const name = (value) => { const address = rva + next - raw; bytes.write(value + "\0", next, "ascii"); next += value.length + 1; return address; };
  if (imports.length) {
    bytes.writeUInt32LE(rva, opt + 120); bytes.writeUInt32LE((imports.length + 1) * 20, opt + 124);
    imports.forEach((value, i) => bytes.writeUInt32LE(name(value), raw + i * 20 + 12));
  }
  if (delay.length) {
    bytes.writeUInt32LE(rva + 0x100, opt + 112 + 13 * 8); bytes.writeUInt32LE((delay.length + 1) * 32, opt + 116 + 13 * 8);
    delay.forEach((value, i) => {
      bytes.writeUInt32LE(legacyDelay ? 0 : 1, raw + 0x100 + i * 32);
      bytes.writeUInt32LE(name(value) + (legacyDelay ? 0x400000 : 0), raw + 0x104 + i * 32);
    });
  }
  return bytes;
}
function runtime() {
  const staging = resolve("qa-model/stage"), files = new Map();
  const inputs = {
    "msvcp140.dll": pe(["vcruntime140.dll", "vcruntime140_1.dll"]),
    "vcruntime140.dll": pe(["kernel32.dll", "api-ms-win-crt-runtime-l1-1-0.dll"]),
    "vcruntime140_1.dll": pe(["vcruntime140.dll"]),
  };
  const manifest = { schema: 1, requirement: "WIN-QA-CRT-01", sourceSha: source, sourceGitStatus: [], toolsetVersion: "14.44.35207",
    stagingDirectory: staging, manifestPath: join(staging, "windows-qa-vc-runtime.json"), files: [] };
  const configuration = { bundle: { resources: { [manifest.manifestPath]: "licenses/windows-qa-vc-runtime.json" } } };
  for (const [name, bytes] of Object.entries(inputs)) {
    const path = join(staging, name); files.set(path, bytes); configuration.bundle.resources[path] = name;
    manifest.files.push({ name, stagedPath: path, bytes: bytes.length, sha256: sha256(bytes), version: "14.44.35211.0", isDebug: false, signature: { ...publisher } });
  }
  return { manifest, configuration, files };
}
function verify(fixture, extra = {}) {
  const manifestBytes = Buffer.from(JSON.stringify(fixture.manifest));
  return verifyRuntime({ manifestBytes, configuration: fixture.configuration, expectedManifestSha256: sha256(manifestBytes), expectedSource: source,
    readFile: (path) => { if (!fixture.files.has(path)) throw new Error("Missing staged/payload file"); return fixture.files.get(path); },
    executableBytes: pe(["kernel32.dll", "MSVCP140.dll"]), ...extra });
}
function temporary() {
  const parent = join(root, "src-tauri/target/windows-qa-crt-test-fixtures"); mkdirSync(parent, { recursive: true });
  const path = mkdtempSync(join(parent, "case-"));
  return { path, cleanup() { const full = resolve(path); if (!full.startsWith(resolve(parent) + sep)) throw new Error("Unsafe fixture cleanup"); rmSync(full, { recursive: true, force: true }); } };
}

describe("Windows QA app-local CRT deployment", () => {
  it("rejects the original QA loader surface with no app-local CRT", () => {
    expect(() => verifyLocalRuntimeClosure(inspectPe(pe(["MSVCP140.dll"])), new Map())).toThrow("Missing app-local dependency msvcp140.dll");
  });
  it("keeps the default OS-only executable independent of CRT staging", () => {
    expect(verifyLocalRuntimeClosure(inspectPe(pe(["USER32.dll", "kernel32.dll"])), new Map())).toEqual([]);
  });
  it("accepts complete CRT files and checks recursive dependencies case-insensitively", () => {
    expect(verify(runtime()).required).toEqual(["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"]);
  });
  it("rejects a missing recursive runtime rather than using the host installation", () => {
    const libraries = new Map([["msvcp140.dll", inspectPe(pe(["vcruntime140.dll"]))]]);
    expect(() => verifyLocalRuntimeClosure(inspectPe(pe(["msvcp140.dll"])), libraries)).toThrow("Missing app-local dependency vcruntime140.dll");
  });
  it.each([false, true])("follows delay imports with legacy VA form %s", (legacyDelay) => {
    const image = inspectPe(pe([], ["MSVCP140.dll"], { legacyDelay }));
    expect(image.delayImports).toEqual(["msvcp140.dll"]);
    expect(() => verifyLocalRuntimeClosure(image, new Map())).toThrow("Missing app-local dependency msvcp140.dll");
  });
  it("handles a cyclic CRT import graph without recursion failure", () => {
    const libs = new Map([["msvcp140.dll", inspectPe(pe(["vcruntime140.dll"]))], ["vcruntime140.dll", inspectPe(pe(["MSVCP140.dll"]))]]);
    expect(verifyLocalRuntimeClosure(inspectPe(pe(["msvcp140.dll"])), libs)).toEqual(["msvcp140.dll", "vcruntime140.dll"]);
  });
  it("rejects an undeclared non-OS DLL", () => {
    expect(() => verifyLocalRuntimeClosure(inspectPe(pe(["mystery.dll"])), new Map())).toThrow("Missing app-local dependency mystery.dll");
  });
  it.each(["subdir/msvcp140.dll", "../msvcp140.dll", "C:\\msvcp140.dll"])("rejects CRT placement %s outside the EXE directory", (target) => {
    const f = runtime(); f.configuration.bundle.resources[f.manifest.files[0].stagedPath] = target;
    expect(() => verify(f)).toThrow("Invalid DLL name or path alias");
  });
  it("rejects case aliases that collide at the destination", () => {
    const f = runtime(); f.configuration.bundle.resources[join(f.manifest.stagingDirectory, "alias.dll")] = "MSVCP140.DLL";
    expect(() => verify(f)).toThrow("duplicate CRT resource");
  });
  it("rejects incomplete core CRT resources", () => {
    const f = runtime(); f.manifest.files.pop(); delete f.configuration.bundle.resources[join(f.manifest.stagingDirectory, "vcruntime140_1.dll")];
    expect(() => verify(f)).toThrow("Core CRT files/resources are incomplete");
  });
  it("rejects a DLL changed after provenance was written", () => {
    const f = runtime(), path = f.manifest.files[0].stagedPath; const damaged = Buffer.from(f.files.get(path)); damaged[1000] ^= 1; f.files.set(path, damaged);
    expect(() => verify(f)).toThrow("CRT file hash mismatch");
  });
  it("rejects a changed provenance file using its separately captured digest", () => {
    expect(() => verify(runtime(), { expectedManifestSha256: "0".repeat(64) })).toThrow("Runtime provenance hash mismatch");
  });
  it("rejects provenance from another source", () => {
    const f = runtime(); f.manifest.sourceSha = "c".repeat(40); expect(() => verify(f)).toThrow("source mismatch");
  });
  it("rejects provenance generated from dirty source", () => {
    const f = runtime(); f.manifest.sourceGitStatus = [" M src-tauri/Cargo.toml"]; expect(() => verify(f)).toThrow("requires clean source");
  });
  it.each(["publisher", "signature", "version", "older-patch", "debug"])("rejects invalid CRT %s metadata", (kind) => {
    const f = runtime(), file = f.manifest.files[0];
    if (kind === "publisher") file.signature.subject = "CN=other, O=Contoso, C=US";
    if (kind === "signature") file.signature.status = "HashMismatch";
    if (kind === "version") file.version = "14.43.10000.0";
    if (kind === "older-patch") file.version = "14.44.10000.0";
    if (kind === "debug") file.isDebug = true;
    expect(() => verify(f)).toThrow(/publisher\/signature rejected|version\/debug build rejected/);
  });
  it.each(["14.44.35207.0", "14.45.1.0"])("accepts a runtime at least as new as the compiler: %s", (value) => {
    const f = runtime(); f.manifest.files[0].version = value;
    expect(verify(f).runtimeFiles).toBe(3);
  });
  it("rejects a correctly hashed x86 DLL in an x64 deployment", () => {
    const f = runtime(), file = f.manifest.files[0], bytes = pe([], [], { machine: 0x14c });
    f.files.set(file.stagedPath, bytes); file.sha256 = sha256(bytes); expect(() => verify(f)).toThrow("Expected AMD64 PE");
  });
  it("rejects duplicate CRT manifest records", () => {
    const f = runtime(); f.manifest.files.push({ ...f.manifest.files[0] }); expect(() => verify(f)).toThrow("duplicate CRT file");
  });
  it("rejects a file path escaping the immutable staging directory", () => {
    const f = runtime(); f.manifest.files[0].stagedPath = join(f.manifest.stagingDirectory, "../msvcp140.dll"); expect(() => verify(f)).toThrow("escaped staging directory");
  });
  it("checks the deployed directory instead of accepting staged copies alone", () => {
    expect(() => verify(runtime(), { payloadRoot: resolve("payload-model") })).toThrow("Missing staged/payload file");
  });
  it("accepts byte-identical deployed CRT and provenance", () => {
    const f = runtime(), payloadRoot = resolve("payload-model");
    for (const file of f.manifest.files) f.files.set(join(payloadRoot, file.name), f.files.get(file.stagedPath));
    f.files.set(join(payloadRoot, "licenses/windows-qa-vc-runtime.json"), Buffer.from(JSON.stringify(f.manifest)));
    expect(verify(f, { payloadRoot }).payloadVerified).toBe(true);
  });
  it.each(["truncate", "unmapped", "name-path", "unterminated"])("rejects malformed PE %s", (kind) => {
    let bytes = pe(["msvcp140.dll"]);
    if (kind === "truncate") bytes = bytes.subarray(0, 200);
    if (kind === "unmapped") bytes.writeUInt32LE(0xffff0000, 0x20c);
    if (kind === "name-path") bytes = pe(["../msvcp140.dll"]);
    if (kind === "unterminated") bytes.writeUInt32LE(20, 0x98 + 124);
    expect(() => inspectPe(bytes)).toThrow();
  });
  it("returns a native CLI failure without emitting a success record", () => {
    const temp = temporary();
    try {
      const manifestPath = join(temp.path, "PROVENANCE.json"), configPath = join(temp.path, "config.json");
      const f = runtime(); f.manifest.stagingDirectory = temp.path; f.manifest.manifestPath = manifestPath;
      const bytes = Buffer.from(JSON.stringify(f.manifest)); writeFileSync(manifestPath, bytes); writeFileSync(configPath, JSON.stringify(f.configuration));
      const result = spawnSync(process.execPath, [join(root, "scripts/verify-windows-qa-runtime.mjs"), "--manifest", manifestPath, "--config", configPath,
        "--manifest-sha256", sha256(bytes), "--expected-source", source], { encoding: "utf8", timeout: 20_000 });
      expect(result.error).toBeUndefined(); expect(result.status).toBe(1); expect(result.stdout).toBe(""); expect(result.stderr).toContain("provenance resource is incomplete");
    } finally { temp.cleanup(); }
  });
});

describe.skipIf(process.platform !== "win32")("actual PowerShell CRT preparation with controlled publisher metadata", { timeout: 30_000 }, () => {
  it.each(shells.flatMap((shell) => ["valid", "valid-inherited-modules", "valid-exact-version", "publisher", "version", "older-patch", "missing-core", "x86"].map((kind) => [shell, kind])))("%s checks %s before publishing a configuration", (shell, kind) => {
    const temp = temporary();
    try {
      const vs = join(temp.path, "VS"), output = join(temp.path, "output"), auxiliary = join(vs, "VC/Auxiliary/Build"); mkdirSync(auxiliary, { recursive: true });
      writeFileSync(join(auxiliary, "Microsoft.VCToolsVersion.default.txt"), "14.44.35207");
      const redist = join(vs, "VC/Redist/MSVC/14.44.35112/x64/Microsoft.VC143.CRT"); mkdirSync(redist, { recursive: true });
      for (const name of ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"]) {
        if (kind === "missing-core" && name === "msvcp140.dll") continue;
        writeFileSync(join(redist, name), pe(name === "msvcp140.dll" ? ["vcruntime140.dll"] : ["kernel32.dll"], [], { machine: kind === "x86" ? 0x14c : 0x8664 }));
      }
      const command = `
        $ErrorActionPreference = 'Stop'
        . $env:CLIPPY_QA_RUNTIME_SCRIPT
        function Get-WindowsQaRuntimeIdentity {
          param([string]$Path)
          [pscustomobject]@{Version=$env:CLIPPY_QA_FAKE_VERSION; IsDebug=$false;
            Signature=[ordered]@{status='Valid'; subject=$env:CLIPPY_QA_FAKE_PUBLISHER; thumbprint=('b'*40)}}
        }
        try {
          New-WindowsQaRuntime -VisualStudioRoot $env:CLIPPY_QA_FAKE_VS -Output $env:CLIPPY_QA_FAKE_OUTPUT -SourceSha ('a'*40) | ConvertTo-Json -Depth 4
        } catch {
          [Console]::Error.WriteLine($_.ToString() + [Environment]::NewLine + $_.ScriptStackTrace)
          exit 1
        }
      `;
      // Windows PowerShell 使用自己的模块目录，避免继承 pwsh 7 的 PSModulePath。
      const shellEnvironment = kind === "valid-inherited-modules" ? process.env
        : Object.fromEntries(Object.entries(process.env).filter(([key]) => key.toLowerCase() !== "psmodulepath"));
      const result = spawnSync(shell, ["-NoProfile", "-NonInteractive", "-Command", command], { encoding: "utf8", timeout: 20_000,
        env: { ...shellEnvironment, CLIPPY_QA_RUNTIME_SCRIPT: join(root, "scripts/prepare-windows-qa-runtime.ps1"), CLIPPY_QA_FAKE_VS: vs,
          CLIPPY_QA_FAKE_OUTPUT: output, CLIPPY_QA_FAKE_VERSION: kind === "version" ? "14.43.10000.0"
            : kind === "older-patch" ? "14.44.10000.0" : kind === "valid-exact-version" ? "14.44.35207.0" : "14.44.35211.0",
          CLIPPY_QA_FAKE_PUBLISHER: kind === "publisher" ? "CN=Other, O=Contoso, C=US" : publisher.subject } });
      const valid = kind.startsWith("valid");
      expect(result.error).toBeUndefined(); expect(result.status, result.stderr).toBe(valid ? 0 : 1);
      expect(existsSync(join(output, "tauri.windows.qa-runtime.conf.json"))).toBe(valid);
      if (valid) {
        const prepared = JSON.parse(result.stdout);
        expect(prepared.runtimeFiles).toBe(3);
        expect(sha256(readFileSync(prepared.manifestPath))).toBe(prepared.manifestSha256);
        const config = JSON.parse(readFileSync(prepared.configPath, "utf8"));
        expect(Object.values(config.bundle.resources)).toContain("msvcp140.dll");
        expect(Object.values(config.bundle.resources)).not.toContain("../msvcp140.dll");
      }
    } finally { temp.cleanup(); }
  });
});

describe("WIN-QA-MSI-PROVENANCE-01 canonical deployment basename", () => {
  it("accepts a canonical staging manifest without requiring installer resource renaming", () => {
    const f = runtime(), previous = f.manifest.manifestPath;
    f.manifest.manifestPath = join(f.manifest.stagingDirectory, "windows-qa-vc-runtime.json");
    delete f.configuration.bundle.resources[previous];
    f.configuration.bundle.resources[f.manifest.manifestPath] = "licenses/windows-qa-vc-runtime.json";
    expect(verify(f).runtimeFiles).toBe(3);
  });
  it("rejects a source basename that MSI would deploy under the legacy name", () => {
    const f = runtime(), previous = f.manifest.manifestPath;
    f.manifest.manifestPath = join(f.manifest.stagingDirectory, "PROVENANCE.json");
    delete f.configuration.bundle.resources[previous];
    f.configuration.bundle.resources[f.manifest.manifestPath] = "licenses/windows-qa-vc-runtime.json";
    expect(() => verify(f)).toThrow("Runtime provenance resource is incomplete");
  });
});

describe.skipIf(process.platform !== "win32")("WIN-QA-MSI-PROVENANCE-01 actual staging filename", { timeout: 30_000 }, () => {
  it.each(shells)("%s stages the same basename that both installers deploy", (shell) => {
    const temp = temporary();
    try {
      const vs = join(temp.path, "VS"), output = join(temp.path, "output"), auxiliary = join(vs, "VC/Auxiliary/Build");
      mkdirSync(auxiliary, { recursive: true });
      writeFileSync(join(auxiliary, "Microsoft.VCToolsVersion.default.txt"), "14.44.35207");
      const redist = join(vs, "VC/Redist/MSVC/14.44.35112/x64/Microsoft.VC143.CRT");
      mkdirSync(redist, { recursive: true });
      for (const name of ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"]) {
        writeFileSync(join(redist, name), pe(name === "msvcp140.dll" ? ["vcruntime140.dll"] : ["kernel32.dll"]));
      }
      // 仅模拟签名/版本元数据，其余调用实际准备器和生产PE验证器，不加载夹具DLL。
      const command = `
        $ErrorActionPreference = 'Stop'
        . $env:CLIPPY_QA_RUNTIME_SCRIPT
        function Get-WindowsQaRuntimeIdentity {
          param([string]$Path)
          [pscustomobject]@{Version='14.44.35211.0'; IsDebug=$false;
            Signature=[ordered]@{status='Valid'; subject='CN=Microsoft publisher, O=Microsoft Corporation, C=US'; thumbprint=('b'*40)}}
        }
        New-WindowsQaRuntime -VisualStudioRoot $env:CLIPPY_QA_FAKE_VS -Output $env:CLIPPY_QA_FAKE_OUTPUT -SourceSha ('a'*40) | ConvertTo-Json -Depth 4
      `;
      const environment = Object.fromEntries(Object.entries(process.env).filter(([key]) => key.toLowerCase() !== "psmodulepath"));
      const result = spawnSync(shell, ["-NoProfile", "-NonInteractive", "-Command", command], { encoding: "utf8", timeout: 20_000,
        env: { ...environment, CLIPPY_QA_RUNTIME_SCRIPT: join(root, "scripts/prepare-windows-qa-runtime.ps1"),
          CLIPPY_QA_FAKE_VS: vs, CLIPPY_QA_FAKE_OUTPUT: output } });
      expect(result.error).toBeUndefined();
      expect(result.status, result.stderr).toBe(0);
      const prepared = JSON.parse(result.stdout), config = JSON.parse(readFileSync(prepared.configPath, "utf8"));
      expect(prepared.manifestPath).toBe(join(prepared.stagingDirectory ?? JSON.parse(readFileSync(prepared.manifestPath, "utf8")).stagingDirectory, "windows-qa-vc-runtime.json"));
      expect(config.bundle.resources[prepared.manifestPath]).toBe("licenses/windows-qa-vc-runtime.json");
      expect(sha256(readFileSync(prepared.manifestPath))).toBe(prepared.manifestSha256);
      expect(existsSync(join(resolve(prepared.manifestPath, ".."), "PROVENANCE.json"))).toBe(false);
    } finally { temp.cleanup(); }
  });
});
