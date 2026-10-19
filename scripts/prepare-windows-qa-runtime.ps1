[CmdletBinding()]
param([string]$OutputDirectory = '')
$ErrorActionPreference = 'Stop'
# 显式选择当前 shell 的模块，兼容从 pwsh 7 启动 Windows PowerShell 5 的模块路径继承。
foreach ($module in @('Microsoft.PowerShell.Utility', 'Microsoft.PowerShell.Security')) {
    Import-Module (Join-Path $PSHOME "Modules\$module\$module.psd1") -ErrorAction Stop
}
$taskRepository = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskRuntimeChecker = Join-Path $PSScriptRoot 'verify-windows-qa-runtime.mjs'
$taskAllowedRuntime = @(
    'concrt140.dll', 'msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll',
    'msvcp140_atomic_wait.dll', 'msvcp140_codecvt_ids.dll', 'vccorlib140.dll',
    'vcruntime140.dll', 'vcruntime140_1.dll', 'vcruntime140_threads.dll'
)
$taskAllowedCrtFamilies = @(
    'Microsoft.VC140.CRT', 'Microsoft.VC141.CRT', 'Microsoft.VC142.CRT',
    'Microsoft.VC143.CRT', 'Microsoft.VC145.CRT'
)
$taskUtf8 = New-Object System.Text.UTF8Encoding($false)

function Get-WindowsQaRuntimeIdentity([string]$Path) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    $version = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
    return [pscustomobject]@{
        Version = $version.FileVersion
        IsDebug = $version.IsDebug
        Signature = [ordered]@{
            status = $signature.Status.ToString()
            subject = $(if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { '' })
            thumbprint = $(if ($signature.SignerCertificate) { $signature.SignerCertificate.Thumbprint } else { '' })
        }
    }
}
function Get-WindowsQaRuntimeHash([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Assert-WindowsQaOutputPath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'QA runtime output path is empty.' }
    $full = [System.IO.Path]::GetFullPath($Path)
    $targetRoot = [System.IO.Path]::GetFullPath((Join-Path $taskRepository 'src-tauri\target'))
    $prefix = $targetRoot.TrimEnd('\') + '\'
    if (-not $full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'QA runtime output must remain inside the repository target directory.'
    }
    # 不经由已有 junction/symlink 把 SDK 文件写到仓库外。
    $parent = $full
    while ($parent -and $parent.Length -gt $taskRepository.Length) {
        if (Test-Path -LiteralPath $parent) {
            if ((Get-Item -LiteralPath $parent -Force).Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'QA runtime output cannot traverse a reparse point.'
            }
        }
        $parent = [System.IO.Path]::GetDirectoryName($parent)
    }
    return $full
}
function New-WindowsQaRuntime {
    param([string]$VisualStudioRoot, [string]$Output, [string]$SourceSha)
    if ($SourceSha -notmatch '^[a-f0-9]{40}$') { throw 'Expected a complete source SHA.' }
    $outputRoot = Assert-WindowsQaOutputPath $Output
    $toolsetText = (Get-Content -LiteralPath (Join-Path $VisualStudioRoot 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt') -Raw).Trim()
    if ($toolsetText -notmatch '^14\.\d+\.\d+$') { throw 'Expected an MSVC v14 toolset.' }
    $toolset = [version]$toolsetText
    $minimumRuntimeVersion = [version]($toolsetText + '.0')
    $redistBase = Join-Path $VisualStudioRoot 'VC\Redist\MSVC'
    # 家族标签不是 ABI/签名证明；仅查找已发布 desktop x64 家族，再执行原文件级校验。
    $choices = @(foreach ($directory in Get-ChildItem -LiteralPath $redistBase -Directory) {
        if ($directory.Name -notmatch '^14\.\d+\.\d+$' -or
            [version]$directory.Name -lt [version]('14.' + $toolset.Minor + '.0')) { continue }
        $desktop = Join-Path $directory.FullName 'x64'
        if (-not (Test-Path -LiteralPath $desktop -PathType Container)) { continue }
        foreach ($family in Get-ChildItem -LiteralPath $desktop -Directory) {
            if ($taskAllowedCrtFamilies -notcontains $family.Name) { continue }
            if ($family.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'Linked CRT family directory rejected.'
            }
            [pscustomobject]@{ Path = $family.FullName; Version = [version]$directory.Name; Family = $family.Name }
        }
    }) | Sort-Object Version -Descending
    $choices = @($choices)
    if ($choices.Count -eq 0) { throw 'Matching x64 release CRT redist directory is missing.' }
    $latest = @($choices | Where-Object { $_.Version -eq $choices[0].Version })
    if ($latest.Count -ne 1) { throw 'Ambiguous latest x64 release CRT directories.' }
    $redist = $latest[0].Path
    $files = @(Get-ChildItem -LiteralPath $redist -File -Filter '*.dll' | Sort-Object Name)
    $names = @($files | ForEach-Object { $_.Name.ToLowerInvariant() })
    foreach ($core in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
        if ($names -notcontains $core) { throw "Required CRT file missing: $core" }
    }
    $records = @()
    foreach ($file in $files) {
        $name = $file.Name.ToLowerInvariant()
        if ($taskAllowedRuntime -notcontains $name -or ($file.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw "Unknown or linked CRT file rejected: $name"
        }
        $identity = Get-WindowsQaRuntimeIdentity $file.FullName
        if ($identity.Signature.status -ne 'Valid' -or
            $identity.Signature.subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)' -or
            $identity.Signature.thumbprint -notmatch '^[a-f0-9]{40}$') {
            throw "Microsoft signature rejected: $name"
        }
        $fileVersion = [version]$identity.Version
        if ($identity.IsDebug -or $fileVersion.Major -ne $toolset.Major -or $fileVersion -lt $minimumRuntimeVersion) {
            throw "CRT version/debug build rejected: $name"
        }
        $bytes = [System.IO.File]::ReadAllBytes($file.FullName)
        if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw "Invalid CRT DOS header: $name" }
        $pe = [BitConverter]::ToUInt32($bytes, 0x3c)
        if ($pe + 26 -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes, $pe) -ne 0x00004550 -or
            [BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664 -or [BitConverter]::ToUInt16($bytes, $pe + 24) -ne 0x20b) {
            throw "Expected AMD64 release CRT: $name"
        }
        $records += [ordered]@{ name = $name; originalPath = $file.FullName; bytes = $bytes.Length
            sha256 = Get-WindowsQaRuntimeHash $file.FullName; version = $identity.Version
            isDebug = $identity.IsDebug; signature = $identity.Signature }
    }
    $stage = Join-Path $outputRoot ('stage-' + [guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $stage -Force)
    $resources = [ordered]@{}
    foreach ($record in $records) {
        $destination = Join-Path $stage $record.name
        Copy-Item -LiteralPath $record.originalPath -Destination $destination
        if ((Get-WindowsQaRuntimeHash $destination) -ne $record.sha256) { throw 'CRT changed during staging.' }
        $record['stagedPath'] = $destination
        $resources[$destination] = $record.name
    }
    # MSI资源文件名取源basename；与最终部署清单名称一致。
    $manifestPath = Join-Path $stage 'windows-qa-vc-runtime.json'
    $manifest = [ordered]@{ schema = 1; requirement = 'WIN-QA-CRT-01'; sourceSha = $SourceSha
        sourceGitStatus = @(); toolsetVersion = $toolsetText; redistDirectory = $redist; redistFamily = $latest[0].Family
        stagingDirectory = $stage; manifestPath = $manifestPath; files = @($records)
        scope = 'QA app-local release CRT; file inspection only; no system installation or DLL load' }
    [System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), $taskUtf8)
    $manifestHash = Get-WindowsQaRuntimeHash $manifestPath
    $resources[$manifestPath] = 'licenses/windows-qa-vc-runtime.json'
    $configPath = Join-Path $stage 'tauri.windows.qa-runtime.conf.json'
    [System.IO.File]::WriteAllText($configPath, ([ordered]@{
        bundle = [ordered]@{ resources = $resources }
    } | ConvertTo-Json -Depth 6), $taskUtf8)
    $verification = & node.exe $taskRuntimeChecker --manifest $manifestPath --config $configPath --manifest-sha256 $manifestHash --expected-source $SourceSha
    if ($LASTEXITCODE -ne 0) { throw 'Staged CRT dependency contract failed.' }
    $result = ($verification -join "`n") | ConvertFrom-Json
    if ($result.runtimeFiles -ne $records.Count) { throw 'Runtime verification output is incomplete.' }
    # 只有全部文件、递归依赖与配置都通过后才发布固定配置入口。
    $publishedConfig = Join-Path $outputRoot 'tauri.windows.qa-runtime.conf.json'
    $temporaryConfig = Join-Path $outputRoot ('config-' + [guid]::NewGuid().ToString('N') + '.tmp')
    Copy-Item -LiteralPath $configPath -Destination $temporaryConfig
    [void](Assert-WindowsQaOutputPath -Path $temporaryConfig)
    [void](Assert-WindowsQaOutputPath -Path $publishedConfig)
    Move-Item -LiteralPath $temporaryConfig -Destination $publishedConfig -Force
    return [pscustomobject]@{ requirement = 'WIN-QA-CRT-01'; sourceSha = $SourceSha
        manifestPath = $manifestPath; manifestSha256 = $manifestHash; configPath = $publishedConfig
        runtimeFiles = $records.Count; toolsetVersion = $toolsetText; scope = $manifest.scope }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        if ($env:OS -ne 'Windows_NT') { throw 'Windows QA runtime preparation requires Windows.' }
        $node = Get-Command node.exe -ErrorAction Stop
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($LASTEXITCODE -ne 0 -or -not $installation) { throw 'Visual Studio C++ tools installation unavailable.' }
        $source = (& git.exe -C $taskRepository rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'Source SHA unavailable.' }
        $status = @(& git.exe -C $taskRepository status --porcelain --untracked-files=all)
        if ($LASTEXITCODE -ne 0 -or $status.Count -ne 0) { throw 'QA runtime provenance requires a clean checkout.' }
        $output = $(if ($OutputDirectory) { $OutputDirectory } else { Join-Path $taskRepository 'src-tauri\target\windows-qa-runtime' })
        $prepared = New-WindowsQaRuntime -VisualStudioRoot $installation.Trim() -Output $output -SourceSha $source
        if ($env:GITHUB_OUTPUT) {
            @("runtime_config=$($prepared.configPath)", "runtime_manifest=$($prepared.manifestPath)",
              "runtime_manifest_sha256=$($prepared.manifestSha256)") | Add-Content -LiteralPath $env:GITHUB_OUTPUT -Encoding UTF8
        }
        $prepared | ConvertTo-Json -Depth 4
        exit 0
    } catch {
        Write-Error "Windows QA runtime preparation failed: $_" -ErrorAction Continue
        exit 1
    }
}
