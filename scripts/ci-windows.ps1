[CmdletBinding()]
param(
    [switch]$FrontendOnly,
    [switch]$RecordingQa,
    [switch]$Quick
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$script:passed = 0
$script:failed = 0
$script:skipped = 0

function Invoke-Check {
    param([string]$Name, [string]$Directory, [scriptblock]$Command)
    Write-Host "[RUN] $Name"
    Push-Location -LiteralPath $Directory
    try {
        $global:LASTEXITCODE = 0
        & $Command
        if ($LASTEXITCODE -ne 0) { throw "Command exited with $LASTEXITCODE" }
        $script:passed++
        Write-Host "[PASS] $Name"
    } catch {
        $script:failed++
        Write-Host "[FAIL] ${Name}: $_"
    } finally {
        Pop-Location
    }
}

function Skip-Check {
    param([string]$Name)
    $script:skipped++
    Write-Host "[SKIP] $Name"
}

function Require-Command {
    param([string]$Name, [string]$Hint)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Missing ${Name}: $Hint"
    }
}

try {
    if ($env:OS -ne 'Windows_NT') { throw 'Run this gate on native Windows.' }
    if ($FrontendOnly -and $RecordingQa) { throw 'FrontendOnly cannot include RecordingQa.' }
    Require-Command node.exe 'Install Node.js >= 22.12.'
    Require-Command npm.cmd 'Install Node.js >= 22.12.'
    $nodeVersion = & node.exe -p 'process.versions.node'
    if ($LASTEXITCODE -ne 0 -or [version]$nodeVersion -lt [version]'22.12.0') {
        throw 'Node.js >= 22.12 is required.'
    }
    if (-not $FrontendOnly) {
        Require-Command python.exe 'Install Python 3; WindowsApps aliases are insufficient.'
        & python.exe -c 'import sys; assert sys.version_info.major == 3'
        if ($LASTEXITCODE -ne 0) { throw 'A working Python 3 interpreter is required.' }
        Require-Command cargo.exe 'Install Rust MSVC and Visual Studio C++ Build Tools + Windows SDK.'
        Require-Command rustc.exe 'Install Rust MSVC.'
        $rustInfo = & rustc.exe -vV
        if ($LASTEXITCODE -ne 0 -or -not ($rustInfo -match 'host: .*windows-msvc')) {
            throw 'The native Rust MSVC host toolchain is required.'
        }
    }
    if ($RecordingQa) {
        foreach ($command in @('sh.exe', 'make.exe', 'diff.exe', 'perl.exe', 'nasm.exe', 'msbuild.exe', 'cmake.exe')) {
            Require-Command $command 'See docs/windows-development.md for the recording source-build toolchain.'
        }
        $clangCommand = Get-Command clang.exe -ErrorAction SilentlyContinue
        $libclangDirectory = if ($env:LIBCLANG_PATH) {
            $env:LIBCLANG_PATH
        } elseif ($clangCommand) {
            Split-Path -Parent $clangCommand.Source
        } else {
            throw 'Install LLVM libclang and set LIBCLANG_PATH to its bin directory.'
        }
        if (-not (Test-Path -LiteralPath (Join-Path $libclangDirectory 'libclang.dll') -PathType Leaf)) {
            throw 'libclang.dll is required for the VP9 source-build bindings; set LIBCLANG_PATH.'
        }
        Require-Command rustup.exe 'Install rustup and llvm-tools-preview.'
        $components = & rustup.exe component list --installed
        if ($LASTEXITCODE -ne 0 -or -not ($components -match '^llvm-tools')) {
            throw 'Install llvm-tools-preview using rustup component add llvm-tools-preview.'
        }
    }
} catch {
    Write-Host "[FAIL] Prerequisites: $_"
    exit 1
}

$scope = if ($FrontendOnly) { 'frontend only (partial)' } else { 'native default + shared checks' }
if ($RecordingQa) { $scope += ' + recording-windows-av-qa' }
if ($Quick) { $scope += ' (production build skipped)' }
Write-Host "Windows gate scope: $scope"
Write-Host 'Linux WebKit/Xvfb pixel smoke, installers and desktop QA require separate evidence.'

$backend = Join-Path $repository 'src-tauri'
$frontend = Join-Path $repository 'src'

if (-not $FrontendOnly) {
    Invoke-Check 'OCR quality contract' $repository {
        & python.exe -m unittest discover -s src-tauri/ocr-sidecar -p 'test_quality*.py' -v
    }
    Invoke-Check 'OCR visual paragraph fallback' $repository {
        & python.exe -m unittest discover -s src-tauri/ocr-sidecar -p test_visual_paragraphs.py -v
    }
    Invoke-Check 'Smart erase feasibility evidence' $repository { & python.exe scripts/smart-erase/verify_evidence.py }
    Invoke-Check 'Rust format' $backend { & cargo.exe fmt -- --check }
    Invoke-Check 'Vendored xcap format' $backend { & cargo.exe fmt --manifest-path vendor/xcap/Cargo.toml -- --check }
    Invoke-Check 'Native Rust check' $backend { & cargo.exe check --locked --all-targets }
    Invoke-Check 'Native Rust clippy' $backend { & cargo.exe clippy --locked --all-targets -- -D warnings }
    Invoke-Check 'Vendored WGC clippy' $backend {
        & cargo.exe clippy --locked --manifest-path vendor/xcap/Cargo.toml --lib --tests --features wgc -- -D warnings
    }
    Invoke-Check 'Windows WGC close state tests' $backend {
        & cargo.exe test --locked --manifest-path vendor/xcap/Cargo.toml --lib --features wgc platform::wgc_runtime::tests
    }
    Invoke-Check 'Windows WGC initialization rollback tests' $backend {
        & cargo.exe test --locked --manifest-path vendor/xcap/Cargo.toml --lib --features wgc platform::wgc_init::tests
    }
    Invoke-Check 'Native Rust tests' $backend { & cargo.exe test --locked }
    Invoke-Check 'Windows CF_HTML parser tests' $backend {
        & cargo.exe test --locked -p arboard --lib platform::windows::html::tests
    }
    Invoke-Check 'Windows image decode budget tests' $backend {
        & cargo.exe test --locked -p arboard --lib platform::windows::image_limits::tests
    }
    Invoke-Check 'Windows DIBV5 decode tests' $backend {
        & cargo.exe test --locked -p arboard --lib platform::windows::image_data::
    }
    Invoke-Check 'Windows DIB file view tests' $backend {
        & cargo.exe test --locked -p arboard --lib platform::windows::dib::tests
    }
    if ($RecordingQa) {
        Invoke-Check 'Windows A/V QA check' $backend {
            & cargo.exe check --locked --features recording-windows-av-qa --all-targets
        }
        Invoke-Check 'Windows A/V QA clippy' $backend {
            & cargo.exe clippy --locked --features recording-windows-av-qa --all-targets -- -D warnings
        }
        Invoke-Check 'Windows A/V QA tests' $backend {
            & cargo.exe test --locked --features recording-windows-av-qa
        }
    } else {
        Skip-Check 'Recording QA feature (request -RecordingQa to include it)'
    }
} else {
    Skip-Check 'Python and native Rust checks (FrontendOnly)'
    Skip-Check 'Recording QA feature (FrontendOnly)'
}

Invoke-Check 'Recording encoder benchmark syntax' $repository { & node.exe --check scripts/benchmark-recording-encoders.mjs }
Invoke-Check 'Recording codec supply chain' $repository { & node.exe scripts/verify-recording-codec-supply-chain.mjs }
Invoke-Check 'Vendored xcap patch' $repository { & node.exe scripts/verify-xcap-patch.mjs }
Invoke-Check 'Locked frontend install' $frontend { & npm.cmd ci --prefer-offline --no-audit --no-fund }
Invoke-Check 'HTML and Tauri boundary' $frontend { & npm.cmd run check:html }
Invoke-Check 'IPC contract' $frontend { & npm.cmd run check:ipc }
Invoke-Check 'Vanilla JS static checks' $frontend { & npm.cmd run lint:js }
Invoke-Check 'TypeScript' $frontend { & npm.cmd run typecheck }
Invoke-Check 'Frontend tests' $frontend { & npm.cmd test }
if ($Quick) {
    Skip-Check 'Production build and real entry check (Quick)'
} else {
    Invoke-Check 'Frontend production build' $frontend { & npm.cmd run build }
    Invoke-Check 'Built main entry' $repository { & node.exe scripts/check-built-main.mjs }
}
Skip-Check 'Linux GNOME, X11 and WebKit/Xvfb DOM/Canvas/layout smoke (separate Linux gate)'
Write-Host "Result: $script:passed passed, $script:failed failed, $script:skipped skipped. Scope: $scope"
Write-Host 'Skipped checks are not passes. Same-SHA native CI and actual Windows desktop QA remain separate.'
if ($script:failed -gt 0) { exit 1 }
exit 0
