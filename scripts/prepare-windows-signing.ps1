param(
  [string]$OutputPath = "src-tauri/tauri.windows.signing.conf.json"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$pfxPath = Join-Path $env:RUNNER_TEMP "clippy-signing.pfx"
$cerPath = Join-Path $env:RUNNER_TEMP "clippy-signing.cer"
try {
  $hasCertificate = -not [string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE)
  $hasPassword = -not [string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_PASSWORD)
  if ($hasCertificate -ne $hasPassword) {
    throw "WINDOWS_CERTIFICATE and WINDOWS_CERTIFICATE_PASSWORD must be configured together"
  }

  if ($hasCertificate) {
    [IO.File]::WriteAllBytes(
      $pfxPath,
      [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE)
    )
    $password = ConvertTo-SecureString `
      -String $env:WINDOWS_CERTIFICATE_PASSWORD -AsPlainText -Force
    $imported = Import-PfxCertificate -FilePath $pfxPath `
      -CertStoreLocation Cert:\CurrentUser\My -Password $password
    $certificate = @($imported) |
      Where-Object { $_.HasPrivateKey } |
      Select-Object -First 1
    $signingMode = "certificate"
  }
  else {
    Write-Host "Creating ephemeral self-signed code-signing certificate"
    $selfSignedParams = @{
      Type = "Custom"
      Subject = "CN=Clippy Self-Signed Release"
      FriendlyName = "Clippy ephemeral self-signed release certificate"
      CertStoreLocation = "Cert:\CurrentUser\My"
      Provider = "Microsoft Software Key Storage Provider"
      KeyAlgorithm = "RSA"
      KeyLength = 2048
      HashAlgorithm = "SHA256"
      KeyExportPolicy = "Exportable"
      KeyUsage = "DigitalSignature"
      TextExtension = @(
        "2.5.29.37={text}1.3.6.1.5.5.7.3.3"
        "2.5.29.19={text}"
      )
      NotAfter = (Get-Date).AddYears(1)
    }
    $certificate = New-SelfSignedCertificate @selfSignedParams
    $signingMode = "self-signed"
  }

  if (-not $certificate) {
    throw "No Windows code-signing certificate was created or imported"
  }
  $thumbprint = $certificate.Thumbprint.ToUpperInvariant()
  $certificate = Get-Item "Cert:\CurrentUser\My\$thumbprint"
  if (-not $certificate.HasPrivateKey) {
    throw "The Windows code-signing certificate has no private key"
  }
  $codeSigningEku = @($certificate.EnhancedKeyUsageList) |
    Where-Object { $_.ObjectId -eq "1.3.6.1.5.5.7.3.3" }
  if (-not $codeSigningEku) {
    throw "The Windows certificate is not valid for code signing"
  }
  $now = Get-Date
  if ($certificate.NotBefore -gt $now -or $certificate.NotAfter -le $now) {
    throw "The Windows code-signing certificate is not currently valid"
  }

  if ($signingMode -eq "self-signed") {
    Export-Certificate -Cert $certificate -FilePath $cerPath | Out-Null
    $trustedCertificate = Import-Certificate -FilePath $cerPath `
      -CertStoreLocation Cert:\CurrentUser\TrustedPeople
    if (-not $trustedCertificate -or
        $trustedCertificate.Thumbprint.ToUpperInvariant() -ne $thumbprint) {
      throw "The self-signed certificate was not installed in TrustedPeople"
    }
  }

  "WINDOWS_CERTIFICATE_THUMBPRINT=$thumbprint" |
    Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
  "WINDOWS_SIGNING_MODE=$signingMode" |
    Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append

  $signingConfig = @{
    bundle = @{
      windows = @{
        certificateThumbprint = $thumbprint
        digestAlgorithm = "sha256"
        timestampUrl = "http://timestamp.digicert.com"
        tsp = $true
      }
    }
  }
  $signingConfig | ConvertTo-Json -Depth 4 |
    Set-Content $OutputPath -Encoding utf8
  Write-Host "Prepared Windows signing mode: $signingMode"
}
finally {
  Remove-Item $pfxPath -Force -ErrorAction SilentlyContinue
  Remove-Item $cerPath -Force -ErrorAction SilentlyContinue
}
