// @vitest-environment node
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("../../", import.meta.url));
const pwsh = join(process.env.ProgramFiles || "C:\\Program Files", "PowerShell/7/pwsh.exe");
const shells = ["powershell.exe", ...(existsSync(pwsh) ? [pwsh] : [])];
const core = ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"];
const microsoft = "CN=controlled publisher, O=Microsoft Corporation, C=US";

// 只构造可解析的 PE 元数据，没有 DLL 代码，也不会调用 Windows loader。
function pe(machine = 0x8664) {
  const bytes = Buffer.alloc(2048), header = 0x80, opt = header + 24;
  bytes.write("MZ"); bytes.writeUInt32LE(header, 0x3c); bytes.write("PE\0\0", header, "binary");
  bytes.writeUInt16LE(machine, header + 4); bytes.writeUInt16LE(1, header + 6); bytes.writeUInt16LE(240, header + 20);
  bytes.writeUInt16LE(0x20b, opt); bytes.writeUInt32LE(0x200, opt + 60); bytes.writeUInt32LE(16, opt + 108);
  bytes.writeUInt32LE(0x1000, opt + 240 + 12); bytes.writeUInt32LE(0x600, opt + 240 + 16); bytes.writeUInt32LE(0x200, opt + 240 + 20);
  return bytes;
}
const old = { version: "14.44.35112", family: "Microsoft.VC143.CRT", fileVersion: "14.44.35211.0" };
const current = { version: "14.50.35719", family: "Microsoft.VC145.CRT", fileVersion: "14.50.35720.0" };
const cases = [
  // 旧家族只验证目录标签发现；文件/签名元数据受控，不声称实际旧 SDK 可编译当前产品。
  ...["140", "141", "142"].map((suffix) => {
    const directory = { ...current, family: `Microsoft.VC${suffix}.CRT` };
    return { name: `published VC${suffix} directory label`, toolset: "14.50.35719", directories: [directory], selected: directory };
  }),
  { name: "existing VC143", toolset: "14.44.35207", directories: [old], selected: old },
  { name: "published VC145", toolset: "14.50.35719", directories: [current], selected: current },
  { name: "side-by-side VC143 and VC145", toolset: "14.50.35719", directories: [old, current], selected: current },
  { name: "newer compatible family", toolset: "14.44.35207", directories: [old, current], selected: current },
  { name: "numeric version ordering", toolset: "14.50.8", directories: [
    { ...current, version: "14.50.9", fileVersion: "14.50.9.0" },
    { ...current, version: "14.50.10", fileVersion: "14.50.10.0" },
  ], selected: { ...current, version: "14.50.10" } },
  { name: "ambiguous latest release family", toolset: "14.50.35719", directories: [current, { ...current, family: "Microsoft.VC143.CRT" }] },
  { name: "unknown family", toolset: "14.50.35719", directories: [{ ...current, family: "Microsoft.VC146.CRT" }] },
  { name: "onecore only", toolset: "14.50.35719", directories: [{ ...current, prefix: "onecore/x64" }] },
  { name: "debug only", toolset: "14.50.35719", directories: [{ ...current, family: "Microsoft.VC145.DebugCRT" }] },
  { name: "x86 only", toolset: "14.50.35719", directories: [{ ...current, prefix: "x86" }] },
  { name: "untrusted newest cannot fall back", toolset: "14.44.35207", directories: [old, { ...current, publisher: "CN=other, O=Contoso, C=US" }] },
  { name: "wrong-machine latest release", toolset: "14.50.35719", directories: [{ ...current, machine: 0x14c }] },
];

describe.skipIf(process.platform !== "win32")("actual PowerShell SDK CRT discovery with controlled metadata", { timeout: 30_000 }, () => {
  it.each(shells.flatMap((shell) => cases.map((fixture) => [shell, fixture.name, fixture])))("%s: %s", (shell, _name, fixture) => {
    const parent = join(root, "src-tauri/target/windows-qa-crt-discovery-fixtures"); mkdirSync(parent, { recursive: true });
    const temporary = mkdtempSync(join(parent, "case-"));
    try {
      const vs = join(temporary, "VS"), output = join(temporary, "output"), auxiliary = join(vs, "VC/Auxiliary/Build");
      mkdirSync(auxiliary, { recursive: true }); writeFileSync(join(auxiliary, "Microsoft.VCToolsVersion.default.txt"), fixture.toolset);
      const metadata = { files: [] };
      for (const directory of fixture.directories) {
        const path = join(vs, "VC/Redist/MSVC", directory.version, directory.prefix ?? "x64", directory.family);
        mkdirSync(path, { recursive: true });
        for (const name of core) {
          const file = join(path, name); writeFileSync(file, pe(directory.machine));
          metadata.files.push({ path: file, version: directory.fileVersion, publisher: directory.publisher ?? microsoft });
        }
      }
      const metadataPath = join(temporary, "METADATA.json"); writeFileSync(metadataPath, JSON.stringify(metadata));
      const command = `
        $ErrorActionPreference = 'Stop'
        . $env:CLIPPY_CRT_DISCOVERY_SCRIPT
        $taskCaseMetadata = [System.IO.File]::ReadAllText($env:CLIPPY_CRT_DISCOVERY_METADATA) | ConvertFrom-Json
        function Get-WindowsQaRuntimeIdentity {
          param([string]$Path)
          $matches = @($taskCaseMetadata.files | Where-Object { $_.path -eq $Path })
          if ($matches.Count -ne 1) { throw 'Controlled metadata file missing' }
          [pscustomobject]@{Version=$matches[0].version; IsDebug=$false;
            Signature=[ordered]@{status='Valid'; subject=$matches[0].publisher; thumbprint=('b'*40)}}
        }
        try {
          New-WindowsQaRuntime -VisualStudioRoot $env:CLIPPY_CRT_DISCOVERY_VS -Output $env:CLIPPY_CRT_DISCOVERY_OUTPUT -SourceSha ('c'*40) | ConvertTo-Json -Depth 4
        } catch {
          [Console]::Error.WriteLine($_.ToString())
          exit 1
        }
      `;
      const result = spawnSync(shell, ["-NoProfile", "-NonInteractive", "-Command", command], { encoding: "utf8", timeout: 20_000,
        env: { ...process.env, CLIPPY_CRT_DISCOVERY_SCRIPT: join(root, "scripts/prepare-windows-qa-runtime.ps1"),
          CLIPPY_CRT_DISCOVERY_METADATA: metadataPath, CLIPPY_CRT_DISCOVERY_VS: vs, CLIPPY_CRT_DISCOVERY_OUTPUT: output } });
      expect(result.error).toBeUndefined(); expect(result.status, result.stderr).toBe(fixture.selected ? 0 : 1);
      expect(existsSync(join(output, "tauri.windows.qa-runtime.conf.json"))).toBe(Boolean(fixture.selected));
      if (fixture.selected) {
        const prepared = JSON.parse(result.stdout), manifest = JSON.parse(readFileSync(prepared.manifestPath, "utf8"));
        const expected = join(vs, "VC/Redist/MSVC", fixture.selected.version, "x64", fixture.selected.family);
        expect(manifest.redistDirectory).toBe(expected);
        expect(prepared.runtimeFiles).toBe(3);
      }
    } finally {
      const full = resolve(temporary);
      if (!full.startsWith(resolve(parent) + sep)) throw new Error("Unsafe discovery fixture cleanup");
      rmSync(full, { recursive: true, force: true });
    }
  });
});
