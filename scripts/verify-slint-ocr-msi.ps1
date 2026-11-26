[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Package,
    [Parameter(Mandatory)][string]$Application
)
$ErrorActionPreference = 'Stop'
$packagePath = (Resolve-Path -LiteralPath $Package).Path
$payload = (New-Item -ItemType Directory -Path (Join-Path $env:RUNNER_TEMP "slint-ocr-$([guid]::NewGuid())")).FullName
try {
    $arguments = @('/a', ('"{0}"' -f $packagePath), ('TARGETDIR="{0}"' -f $payload), '/qn', '/norestart')
    $process = Start-Process msiexec.exe -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "MSI payload extraction failed ($($process.ExitCode))" }

    $applicationFile = @(Get-ChildItem -LiteralPath $payload -Filter $Application -File -Recurse)
    if ($applicationFile.Count -ne 1) { throw "Expected one $Application in the extracted MSI" }
    $bundle = Join-Path $applicationFile[0].DirectoryName 'ocr'
    $verify = Join-Path $PSScriptRoot 'prepare-slint-ocr.py'
    python -B $verify --bundle $bundle
    if ($LASTEXITCODE -ne 0) { throw 'MSI OCR inventory check failed' }
    python -B $verify --bundle $bundle --smoke
    if ($LASTEXITCODE -ne 0) { throw 'MSI OCR smoke test failed' }
} finally {
    Remove-Item -LiteralPath $payload -Recurse -Force
}
