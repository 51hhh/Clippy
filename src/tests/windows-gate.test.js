// @vitest-environment node
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const gate = fileURLToPath(new URL("../../scripts/ci-windows.ps1", import.meta.url));
const nativePrelude = `
  function global:python.exe { $global:LASTEXITCODE = 0 }
  function global:rustc.exe {
    $global:LASTEXITCODE = 0
    'host: x86_64-pc-windows-msvc'
  }
`;

function runGate(failTests, switches = "-FrontendOnly -Quick", prelude = "") {
  // 替代外部命令，验证真实 PowerShell 入口的退出码；不安装依赖、不运行 Rust 或写入产物。
  const command = `
    ${prelude}
    function global:node.exe { $global:LASTEXITCODE = 0 }
    function global:npm.cmd {
      if (${failTests ? "$true" : "$false"} -and $args[0] -eq 'test') {
        $global:LASTEXITCODE = 23
      } else { $global:LASTEXITCODE = 0 }
    }
    & $env:CLIPPY_GATE_SCRIPT ${switches}
  `;
  return spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", command], {
    encoding: "utf8",
    env: { ...process.env, CLIPPY_GATE_SCRIPT: gate },
    timeout: 20_000,
  });
}

// 子进程硬超时为 20 秒，外层合同预算须覆盖 Windows 冷启动；退出码/失败断言保持不变。
describe.skipIf(process.platform !== "win32")("native Windows gate failure accounting", { timeout: 30_000 }, () => {
  it("returns failure when npm test exits nonzero even when later checks succeed", () => {
    const result = runGate(true);
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(1);
    expect(result.stdout).toContain("[FAIL] Frontend tests: Command exited with 23");
    expect(result.stdout).toMatch(/Result: \d+ passed, 1 failed, \d+ skipped/);
  });

  it("labels frontend-only quick execution as partial and reports all skipped groups", () => {
    const result = runGate(false);
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(0);
    expect(result.stdout).toContain("frontend only (partial)");
    expect(result.stdout).toContain("production build skipped");
    expect(result.stdout).toContain("0 failed, 4 skipped");
  });

  it("rejects recording QA combined with frontend-only before executing checks", () => {
    const result = runGate(false, "-FrontendOnly -RecordingQa");
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(1);
    expect(result.stdout).toContain("FrontendOnly cannot include RecordingQa");
    expect(result.stdout).not.toContain("[RUN]");
  });

  it("fails at prerequisites when cargo is missing without reporting successful checks", () => {
    const result = runGate(false, "", `
      function global:python.exe { $global:LASTEXITCODE = 0 }
      function global:Get-Command {
        [CmdletBinding()] param([string]$Name)
        if ($Name -eq 'cargo.exe') { return $null }
        Microsoft.PowerShell.Core\\Get-Command @PSBoundParameters
      }
    `);
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(1);
    expect(result.stdout).toContain("[FAIL] Prerequisites: Missing cargo.exe");
    expect(result.stdout).not.toContain("[RUN]");
    expect(result.stdout).not.toContain("Result:");
  });

  it("requires CMake before starting the recording source build", () => {
    const result = runGate(false, "-RecordingQa", `${nativePrelude}
      function global:Get-Command {
        [CmdletBinding()] param([string]$Name)
        if ($Name -eq 'cmake.exe') { return $null }
        [pscustomobject]@{ Name = $Name; Source = $Name }
      }
    `);
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(1);
    expect(result.stdout).toContain("[FAIL] Prerequisites: Missing cmake.exe");
    expect(result.stdout).not.toContain("[RUN]");
  });

  it("requires libclang.dll separately from the Rust LLVM tools", () => {
    const result = runGate(false, "-RecordingQa", `${nativePrelude}
      $env:LIBCLANG_PATH = Join-Path $env:TEMP ([guid]::NewGuid().ToString())
      function global:Get-Command {
        [CmdletBinding()] param([string]$Name)
        [pscustomobject]@{ Name = $Name; Source = $Name }
      }
    `);
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(1);
    expect(result.stdout).toContain("libclang.dll is required");
    expect(result.stdout).not.toContain("[RUN]");
  });
});
