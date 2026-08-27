<#
.SYNOPSIS
    Windows Authenticode signing hook for tauri-bundler.

.DESCRIPTION
    Wired in through `bundle.windows.signCommand` in tauri.windows.conf.json.
    tauri-bundler invokes this once per produced binary (the app .exe, every
    staged transcribe-cpp .dll, and the NSIS/MSI installers), passing the file
    path as the single argument.

    Why a wrapper instead of calling trusted-signing-cli directly from
    signCommand:

      1. The CLI renamed itself (trusted-signing-cli -> artifact-signing-cli)
         and moved its environment variable names between releases
         (AZURE_CODE_SIGNING_NAME -> AZURE_ARTIFACT_SIGNING_ACCOUNT). Passing
         the values as explicit -e/-a/-c flags — which every release accepts —
         keeps tauri.conf.json decoupled from that churn.

      2. A local `bun run tauri build` has no Azure credentials. Putting the
         CLI directly in signCommand would make every local Windows build fail.
         This script skips signing when no signing endpoint is configured, so
         local builds keep producing (unsigned) artifacts exactly as before.

      3. Skipping silently is only safe if CI cannot ship unsigned artifacts by
         accident. Every signed path is appended to the receipt file named by
         SPEAK_SIGN_RECEIPT; the release workflow asserts that file is
         non-empty, so a missing/expired secret fails the build loudly instead
         of publishing binaries that trip SmartScreen.

.PARAMETER Path
    The binary to sign. Supplied by tauri-bundler via the %1 placeholder.

.NOTES
    Configuration (all read from the environment):
      AZURE_SIGNING_ENDPOINT      e.g. https://weu.codesigning.azure.net
      AZURE_SIGNING_ACCOUNT       Trusted Signing account name
      AZURE_SIGNING_CERT_PROFILE  certificate profile name
      AZURE_CLIENT_ID / AZURE_CLIENT_SECRET / AZURE_TENANT_ID
                                  service principal, read by the CLI itself
      SPEAK_SIGN_RECEIPT          optional; file to append signed paths to
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Path
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path)) {
    Write-Error "sign-windows: file to sign does not exist: $Path"
    exit 2
}

$endpoint = $env:AZURE_SIGNING_ENDPOINT
$account = $env:AZURE_SIGNING_ACCOUNT
$profileName = $env:AZURE_SIGNING_CERT_PROFILE

# No signing configuration at all -> unsigned local build. This is the
# documented, expected path for `bun run tauri build` on a dev machine.
if ([string]::IsNullOrWhiteSpace($endpoint) -and
    [string]::IsNullOrWhiteSpace($account) -and
    [string]::IsNullOrWhiteSpace($profileName)) {
    Write-Host "sign-windows: no AZURE_SIGNING_* configuration, leaving '$Path' unsigned."
    exit 0
}

# Partial configuration is always a mistake — a typo'd secret name would
# otherwise degrade into a silently unsigned release.
$missing = @()
if ([string]::IsNullOrWhiteSpace($endpoint)) { $missing += "AZURE_SIGNING_ENDPOINT" }
if ([string]::IsNullOrWhiteSpace($account)) { $missing += "AZURE_SIGNING_ACCOUNT" }
if ([string]::IsNullOrWhiteSpace($profileName)) { $missing += "AZURE_SIGNING_CERT_PROFILE" }
if ($missing.Count -gt 0) {
    Write-Error ("sign-windows: incomplete signing configuration, missing: " + ($missing -join ", "))
    exit 2
}

$cli = Get-Command "trusted-signing-cli" -ErrorAction SilentlyContinue
if (-not $cli) {
    $cli = Get-Command "artifact-signing-cli" -ErrorAction SilentlyContinue
}
if (-not $cli) {
    Write-Error "sign-windows: neither trusted-signing-cli nor artifact-signing-cli is on PATH (is the 'Install trusted-signing-cli' step present and did cargo's bin dir get added to PATH?)"
    exit 2
}

Write-Host "sign-windows: signing '$Path' via $($cli.Name)"
& $cli.Source -e $endpoint -a $account -c $profileName $Path
if ($LASTEXITCODE -ne 0) {
    Write-Error "sign-windows: signing failed for '$Path' (exit $LASTEXITCODE)"
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($env:SPEAK_SIGN_RECEIPT)) {
    Add-Content -LiteralPath $env:SPEAK_SIGN_RECEIPT -Value $Path -Encoding utf8
}

exit 0
