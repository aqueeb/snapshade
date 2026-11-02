# scripts/sign-windows.ps1 — Tauri `bundle.windows.signCommand` hook (PKG-04).
# Tauri calls this once per file it produces: snapshade.exe, the NSIS plugin DLLs, the NSIS
# installer and, from inside NSIS (`!uninstfinalize`), the uninstaller. Three ways out, checked
# in this order:
#   1. $env:SNAPSHADE_NO_SIGN set           -> leave the file unsigned (forks, dry runs).
#   2. $env:AZURE_ARTIFACT_SIGNING_ACCOUNT  -> Azure Artifact Signing (docs/RELEASING.md):
#      signtool with Microsoft's signing library (Microsoft.ArtifactSigning.Client), which
#      authenticates from AZURE_TENANT_ID / AZURE_CLIENT_ID / AZURE_CLIENT_SECRET (an App
#      Registration holding the "Artifact Signing Certificate Profile Signer" role) and signs
#      with the profile AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE at
#      AZURE_ARTIFACT_SIGNING_ENDPOINT (e.g. https://eus.codesigning.azure.net). The certificate
#      never leaves Azure. The library is fetched once from the NuGet CDN (pinned version and
#      SHA-256) into AZURE_ARTIFACT_SIGNING_CLIENT_DIR, or %LOCALAPPDATA% when that is unset.
#   3. $env:SNAPSHADE_PFX                   -> signtool with a .pfx (password in
#      $env:WINDOWS_CERTIFICATE_PASSWORD). Test certificates only: since June 2023 no CA hands
#      out a downloadable code-signing key.
#   Otherwise the file is left unsigned with a note.
# `-FetchClientOnly` downloads the signing library and exits; the release workflow uses it to
# keep the download out of the build step.
# One identity must be kept across releases so SmartScreen reputation accumulates. Every signed
# file is re-read with Get-AuthenticodeSignature so a "succeeded but unsigned" run fails the
# build instead of shipping.
param([string]$File, [switch]$FetchClientOnly)
$ErrorActionPreference = "Stop"

# Windows PowerShell started from pwsh (GitHub's `shell: pwsh` step -> tauri build -> this hook)
# inherits PowerShell 7's PSModulePath, finds 7's Microsoft.PowerShell.Security first, cannot
# load it and reports "command was found in the module ... but the module could not be loaded"
# for Get-AuthenticodeSignature. Pin this host's own modules first.
if ($PSVersionTable.PSEdition -eq "Desktop") {
  $env:PSModulePath = "$PSHOME\Modules;${env:ProgramFiles}\WindowsPowerShell\Modules"
}
Import-Module Microsoft.PowerShell.Security

# Microsoft.ArtifactSigning.Client: the signtool /dlib library plus its Azure Identity stack.
# Bump the version and the hash together; the package is immutable on NuGet.
$ClientVersion = "1.0.146"
$ClientSha256 = "7761E5D8F31E54B764DA21D33CD8B0A2940E1B05526C7DFCB5E94B0431426F70"
$ClientUrl = "https://api.nuget.org/v3-flatcontainer/microsoft.artifactsigning.client/$ClientVersion/microsoft.artifactsigning.client.$ClientVersion.nupkg"

function Find-Signtool {
  $candidates = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
    Sort-Object FullName -Descending
  if (-not $candidates) { throw "sign-windows: signtool.exe not found (install the Windows 10/11 SDK)" }
  return $candidates[0].FullName
}

function Get-SigningClient {
  # Returns the path of Azure.CodeSigning.Dlib.dll, fetching the package when it is missing.
  $dir = if ($env:AZURE_ARTIFACT_SIGNING_CLIENT_DIR) { $env:AZURE_ARTIFACT_SIGNING_CLIENT_DIR }
         else { Join-Path $env:LOCALAPPDATA "artifact-signing-client\$ClientVersion" }
  $dlib = Join-Path $dir "bin\x64\Azure.CodeSigning.Dlib.dll"
  if (Test-Path -LiteralPath $dlib) { return $dlib }
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $zip = Join-Path $dir "microsoft.artifactsigning.client.zip"
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  for ($i = 1; $i -le 5; $i++) {
    try {
      Write-Host "sign-windows: fetching Microsoft.ArtifactSigning.Client $ClientVersion (attempt $i)"
      Invoke-WebRequest -Uri $ClientUrl -OutFile $zip -UseBasicParsing
      $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash
      if ($hash -ne $ClientSha256) { throw "SHA-256 mismatch: got $hash, expected $ClientSha256" }
      Expand-Archive -LiteralPath $zip -DestinationPath $dir -Force
      Remove-Item -LiteralPath $zip -Force
      if (Test-Path -LiteralPath $dlib) { return $dlib }
      throw "the package did not contain bin\x64\Azure.CodeSigning.Dlib.dll"
    } catch {
      Write-Host "sign-windows: $($_.Exception.Message)"
      if ($i -eq 5) { throw "sign-windows: could not fetch the signing client from $ClientUrl" }
      Start-Sleep -Seconds (5 * $i)
    }
  }
}

function Assert-Signed([string]$Path, [switch]$RequireTrusted) {
  $sig = Get-AuthenticodeSignature -LiteralPath $Path
  if ($sig.Status -eq "NotSigned" -or ($RequireTrusted -and $sig.Status -ne "Valid")) {
    throw "sign-windows: $Path is not validly signed after signing (status $($sig.Status): $($sig.StatusMessage))"
  }
  Write-Host "sign-windows: $Path signed by '$($sig.SignerCertificate.Subject)' (status $($sig.Status), thumbprint $($sig.SignerCertificate.Thumbprint))"
}

if ($FetchClientOnly) {
  $path = Get-SigningClient
  Write-Host "sign-windows: signing client ready at $path"
  exit 0
}

if ($env:SNAPSHADE_NO_SIGN) { Write-Host "sign-windows: skipped (SNAPSHADE_NO_SIGN)"; exit 0 }

if ($env:AZURE_ARTIFACT_SIGNING_ACCOUNT) {
  foreach ($name in "AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET", "AZURE_ARTIFACT_SIGNING_ENDPOINT", "AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE") {
    $item = Get-Item "env:$name" -ErrorAction SilentlyContinue
    if (-not $item -or -not $item.Value) { throw "sign-windows: AZURE_ARTIFACT_SIGNING_ACCOUNT is set but $name is empty" }
  }
  if (-not $env:SIGNTOOL_PATH) { $env:SIGNTOOL_PATH = Find-Signtool }
  $dlib = Get-SigningClient
  # The library reads this file (signtool /dmdf). Only the environment credential, i.e. the three
  # AZURE_* variables, is tried: no az login, no managed-identity or IDE probing per file.
  $metadata = Join-Path ([IO.Path]::GetTempPath()) "snapshade-artifact-signing.json"
  @{
    Endpoint               = $env:AZURE_ARTIFACT_SIGNING_ENDPOINT
    CodeSigningAccountName = $env:AZURE_ARTIFACT_SIGNING_ACCOUNT
    CertificateProfileName = $env:AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE
    ExcludeCredentials     = @(
      "ManagedIdentityCredential", "WorkloadIdentityCredential", "SharedTokenCacheCredential",
      "VisualStudioCredential", "VisualStudioCodeCredential", "AzureCliCredential",
      "AzurePowerShellCredential", "AzureDeveloperCliCredential", "InteractiveBrowserCredential"
    )
  } | ConvertTo-Json | Set-Content -LiteralPath $metadata -Encoding ASCII
  # Three attempts: the signing and timestamp services can hiccup once on a fresh runner.
  $attempt = 0
  do {
    $attempt++
    & $env:SIGNTOOL_PATH sign /v /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 /dlib $dlib /dmdf $metadata /d "Snapshade" $File
    $code = $LASTEXITCODE
    if ($code -ne 0 -and $attempt -lt 3) {
      Write-Host "sign-windows: signtool failed ($code) on attempt $attempt, retrying in 10 s"
      Start-Sleep -Seconds 10
    }
  } while ($code -ne 0 -and $attempt -lt 3)
  if ($code -ne 0) { throw "sign-windows: signtool failed ($code) after $attempt attempts" }
  Assert-Signed $File -RequireTrusted
  exit 0
}

if ($env:SNAPSHADE_PFX) {
  $signtool = Find-Signtool
  & $signtool sign /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com /f $env:SNAPSHADE_PFX /p $env:WINDOWS_CERTIFICATE_PASSWORD $File
  if ($LASTEXITCODE -ne 0) { throw "sign-windows: signtool failed ($LASTEXITCODE)" }
  Assert-Signed $File
  exit 0
}

Write-Host "sign-windows: no AZURE_ARTIFACT_SIGNING_ACCOUNT or SNAPSHADE_PFX, leaving $File unsigned"
exit 0
