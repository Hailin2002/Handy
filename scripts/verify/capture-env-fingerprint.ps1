# capture-env-fingerprint.ps1
#
# Captures a machine-readable verification-environment fingerprint of the
# current host and writes it as a stable JSON snapshot. The snapshot is the
# canonical evidence anchor consumed by all future verification gates: every
# later gate result can be attributed to the exact hardware/toolchain it ran
# on.
#
# Determinism contract (acceptance gate G1): two consecutive captures on an
# unchanged host differ ONLY in the top-level `capturedAt` timestamp. All
# other fields are derived from stable sources and explicitly sorted.
#
# Rerunnable without interaction and WITHOUT elevation (gate G3):
#   pwsh ./scripts/verify/capture-env-fingerprint.ps1
#   pwsh ./scripts/verify/capture-env-fingerprint.ps1 -OutPath <file>
#
# Source policy: everything is captured from sources available to an
# unprivileged user (registry, .NET, environment, toolchain binaries).
# WMI/CIM is used only opportunistically to enrich optional detail fields;
# every such lookup fails soft to $null so restricted hosts still produce a
# complete, parseable snapshot.
#
# Scope note (audio): on Windows, cpal enumerates active WASAPI/MMDevice
# endpoints (render + capture). This script mirrors that enumeration via the
# MMDevices registry view, filtered to DeviceState == 1 (Active), so the
# snapshot lists exactly the endpoints a cpal-based probe would see.
#
# ConstrainedLanguage compatibility: the script completes under
# ConstrainedLanguage in a degraded mode. Blocked .NET constructs are
# replaced or fail soft with an explicit warning instead of a cryptic
# "Cannot create type / Cannot invoke method" error:
#   - Add-Type GlobalMemoryStatusEx P/Invoke -> fails soft; total memory is
#     emitted as null (CIM fallback is NOT used: it is unavailable on
#     restricted hosts and this script already treats CIM as optional).
#   - .NET registry enumeration (audio endpoints, GPU display class) ->
#     fails soft to empty lists; each degraded section is reported.
#   - [IO.File]::WriteAllText + UTF8Encoding::new -> Write-Utf8NoBomFile
#     helper with a Set-Content fallback (Windows PowerShell 5.1 prepends a
#     BOM under ConstrainedLanguage - documented residual limit).
#   - $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath
#     -> cmdlet-based absolute-path fallback.
# A CLM-captured snapshot therefore parses cleanly but may contain null /
# empty detail fields; every degraded field is announced on the host.

[CmdletBinding()]
param(
    # Where the snapshot is written. Defaults beside the other review evidence.
    [string]$OutPath = (Join-Path $PSScriptRoot '..\..\docs\reviews\verification-environment-fingerprint.json')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Degraded-section tracking: every section that had to fall back (CLM-blocked
# .NET call or restricted source) is recorded and announced at the end, so a
# CLM snapshot is never silently less complete than the header claims.
$script:degradedSections = @()
function Add-DegradedSection {
    param([string]$Name, [string]$Reason)
    $script:degradedSections += ('{0} ({1})' -f $Name, $Reason)
}

function Write-Utf8NoBomFile {
    # UTF-8 without BOM writer. Primary path is byte-identical to the previous
    # direct [IO.File]::WriteAllText + UTF8Encoding($false) call; under CLM
    # both constructs are blocked ("Cannot create type / Cannot invoke
    # method") and Set-Content keeps the snapshot on disk. Residual limit:
    # Windows PowerShell 5.1 Set-Content prepends a UTF-8 BOM.
    param([string]$Path, [string]$Text)

    try {
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($Path, $Text, $utf8NoBom)
    }
    catch {
        Add-DegradedSection 'snapshot-file-write' 'ConstrainedLanguage: BOM prepended by Set-Content fallback'
        Set-Content -LiteralPath $Path -Value $Text -Encoding UTF8
    }
}

function Get-ToolVersion {
    # Returns the first stdout line of `<exe> --version`, or $null when the
    # tool is not installed / not on PATH. Never throws.
    param([string]$Exe, [string[]]$VersionArgs = @('--version'))

    $resolved = Get-Command $Exe -ErrorAction SilentlyContinue
    if (-not $resolved) { return $null }
    try {
        $output = & $Exe @VersionArgs 2>$null
        if (-not $output) { return $null }
        $line = ($output | Select-Object -First 1)
        if ($null -eq $line) { return $null }
        return ('{0}' -f $line).Trim()
    } catch {
        return $null
    }
}

function Get-CimOrNull {
    # Opportunistic CIM query: returns $null instead of failing on hosts
    # where WMI/CIM access is restricted.
    param([string]$ClassName)
    try {
        return Get-CimInstance -ClassName $ClassName -ErrorAction Stop
    } catch {
        return $null
    }
}

function Get-AudioEndpoints {
    # Enumerate active MMDevice endpoints of one flow ("Render"/"Capture"),
    # equivalent to cpal's devices()/default_device() on Windows (WASAPI):
    # only DeviceState == 1 (ACTIVE) endpoints are visible to applications.
    #
    # Read via the .NET registry API: PowerShell's recursive PS drive walks
    # hit ACL-restricted subkeys, while explicit OpenSubKey works unprivileged.
    param([string]$Flow)

    # Plain array, not Generic.List[object]: New-Object on generic types is
    # blocked under ConstrainedLanguage, and this call sat OUTSIDE the
    # try/catch below, aborting the whole snapshot instead of degrading.
    $endpoints = @()
    $baseKey = $null
    try {
        $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Default').OpenSubKey("SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\$Flow")
        if (-not $baseKey) { return ,@() }

        foreach ($epId in $baseKey.GetSubKeyNames()) {
            $epKey = $null
            try { $epKey = $baseKey.OpenSubKey($epId) } catch { continue }
            if (-not $epKey) { continue }
            try {
                $state = $epKey.GetValue('DeviceState')
                if ($state -isnot [int] -or $state -ne 1) { continue }   # 1 = ACTIVE

                # Friendly name resolution. The live WASAPI property store
                # exposes PKEY_Device_FriendlyName ({a45c254e...},14); some
                # persisted stores only carry the device description (,2)
                # plus the bus-reported interface name, which combine into
                # the same "Description (Interface)" shape users see.
                $name = $null
                $propsKey = $null
                try { $propsKey = $epKey.OpenSubKey('Properties') } catch { $propsKey = $null }
                if ($propsKey) {
                    try {
                        $friendly   = $propsKey.GetValue('{a45c254e-df1c-4efd-8020-67d146a850e0},14')
                        $deviceDesc = $propsKey.GetValue('{a45c254e-df1c-4efd-8020-67d146a850e0},2')
                        $busDesc    = $propsKey.GetValue('{b3f8fa53-0004-438e-9003-51a46e139bfc},6')
                        if ($friendly) {
                            $name = [string]$friendly
                        } elseif ($deviceDesc -and $busDesc) {
                            $name = ('{0} ({1})' -f [string]$deviceDesc, [string]$busDesc)
                        } elseif ($deviceDesc) {
                            $name = [string]$deviceDesc
                        } elseif ($busDesc) {
                            $name = [string]$busDesc
                        }
                    } finally {
                        $propsKey.Dispose()
                    }
                }

                $endpoints += [ordered]@{
                    id   = $epId
                    name = $name
                }
            } finally {
                $epKey.Dispose()
            }
        }
    } catch {
        # Whole-flow failure degrades to an empty list rather than a broken run
        # (ConstrainedLanguage blocks the .NET registry API entirely; restricted
        # ACLs can do the same). The degradation is recorded, not silent.
        Add-DegradedSection "audio-$Flow" 'enumeration failed (.NET registry API blocked or ACL-restricted)'
        return ,@()
    } finally {
        if ($baseKey) { $baseKey.Dispose() }
    }

    # Sort for the determinism contract.
    return ,@($endpoints | Sort-Object -Property id)
}

function Get-TotalPhysicalMemoryBytes {
    # GlobalMemoryStatusEx: unprivileged, no WMI needed. Under
    # ConstrainedLanguage the Add-Type P/Invoke is blocked; there is no
    # cmdlet-only equivalent that returns the same figure (CIM is optional on
    # restricted hosts and cannot be relied on), so this fails soft to $null
    # and records the degradation instead of aborting the whole snapshot.
    try {
        Add-Type -Namespace Native -Name MemStatus -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)]
public struct MEMORYSTATUSEX {
    public uint dwLength;
    public uint dwMemoryLoad;
    public ulong ullTotalPhys;
    public ulong ullAvailPhys;
    public ulong ullTotalPageFile;
    public ulong ullAvailPageFile;
    public ulong ullTotalVirtual;
    public ulong ullAvailVirtual;
    public ulong ullAvailExtendedVirtual;
}
[DllImport("kernel32.dll", SetLastError = true)]
public static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX lpBuffer);
'@
        $status = New-Object Native.MemStatus+MEMORYSTATUSEX
        $status.dwLength = [uint32][Runtime.InteropServices.Marshal]::SizeOf([type][Native.MemStatus+MEMORYSTATUSEX])
        $null = [Native.MemStatus]::GlobalMemoryStatusEx([ref]$status)
        return $status.ullTotalPhys
    }
    catch {
        Add-DegradedSection 'memory.totalPhysicalBytes' 'ConstrainedLanguage: Add-Type P/Invoke (GlobalMemoryStatusEx) blocked'
        return $null
    }
}

# ---------------------------------------------------------------------------
# Capture
# ---------------------------------------------------------------------------

$capturedAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ')

# --- Operating system -------------------------------------------------------
$osVersion   = [Environment]::OSVersion.Version   # includes UBR, e.g. 10.0.26200.6584
$cv          = Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$buildNumber = if ($cv.PSObject.Properties['CurrentBuildNumber']) { $cv.CurrentBuildNumber } else { '{0}' -f $osVersion.Build }
$ubr         = if ($cv.PSObject.Properties['UBR']) { $cv.UBR } else { $osVersion.Revision }
$displayVersion = if ($cv.PSObject.Properties['DisplayVersion']) { $cv.DisplayVersion } else { $null }
$productName    = if ($cv.PSObject.Properties['ProductName']) { $cv.ProductName } else { $null }
$editionId      = if ($cv.PSObject.Properties['EditionID']) { $cv.EditionID } else { $null }

# ProductName still reads "Windows 10 ..." on Windows 11; normalize by build.
$caption = $productName
if ($caption -and ([int]$buildNumber) -ge 22000) {
    $caption = $caption -replace '^Windows 10', 'Windows 11'
}
$osArch = if ([Environment]::Is64BitOperatingSystem) { '64-bit' } else { '32-bit' }

# --- CPU --------------------------------------------------------------------
$cpuKey = Get-ItemProperty -Path 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0' -ErrorAction SilentlyContinue
$logicalCores = [int]$env:NUMBER_OF_PROCESSORS

# Optional detail via CIM (fails soft to $null on restricted hosts).
$cpuCim = Get-CimOrNull -ClassName 'Win32_Processor' | Select-Object -First 1
$physicalCores     = if ($cpuCim) { [int]$cpuCim.NumberOfCores } else { $null }
$maxClockSpeedMhz  = if ($cpuKey -and $cpuKey.PSObject.Properties['~MHz']) { [int]$cpuKey.'~MHz' }
                     elseif ($cpuCim) { [int]$cpuCim.MaxClockSpeed } else { $null }

# --- Memory -----------------------------------------------------------------
$totalMemBytes = Get-TotalPhysicalMemoryBytes

# --- GPU (display-class drivers from the registry; no WMI required) ---------
# Enumerated via the .NET registry API: the display class key contains a
# heavily ACL-restricted "Properties" subkey that trips recursive PS drives,
# so we open each driver subkey explicitly and skip non-device keys.
$gpus = @()
try {
    $gpuClassKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Default').OpenSubKey('SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}')
    if ($gpuClassKey) {
        # Plain array: Generic.List[object] is not constructible under
        # ConstrainedLanguage; the array form captures GPUs even there.
        $found = @()
        foreach ($sub in $gpuClassKey.GetSubKeyNames()) {
            # Device instances are numeric ("0000", "0001", ...).
            if ($sub -notmatch '^\d{4}$') { continue }
            $dev = $null
            try { $dev = $gpuClassKey.OpenSubKey($sub) } catch { continue }
            if (-not $dev) { continue }
            try {
                $desc = $dev.GetValue('DriverDesc')
                if ($desc) {
                    $found += [ordered]@{
                        name          = [string]$desc
                        driverVersion = if ($dev.GetValue('DriverVersion')) { [string]$dev.GetValue('DriverVersion') } else { $null }
                    }
                }
            } finally {
                $dev.Dispose()
            }
        }
        $gpuClassKey.Dispose()
        $gpus = @($found | Sort-Object -Property name)
    }
} catch {
    $gpus = @()
    Add-DegradedSection 'gpu' 'enumeration failed (.NET registry API blocked or ACL-restricted)'
}

# --- Audio (cpal-equivalent active endpoint enumeration) --------------------
$audio = [ordered]@{
    backend = 'wasapi-mmdevice-active-endpoints'
    render  = Get-AudioEndpoints -Flow 'Render'
    capture = Get-AudioEndpoints -Flow 'Capture'
}

# --- Toolchains -------------------------------------------------------------
$toolchain = [ordered]@{
    bun    = Get-ToolVersion -Exe 'bun'
    node   = Get-ToolVersion -Exe 'node'
    cargo  = Get-ToolVersion -Exe 'cargo'
    rustc  = Get-ToolVersion -Exe 'rustc'
    cmake  = Get-ToolVersion -Exe 'cmake'
    git    = Get-ToolVersion -Exe 'git'
    python = Get-ToolVersion -Exe 'python'
}

$fingerprint = [ordered]@{
    schemaVersion = 1
    capturedAt    = $capturedAt
    generator     = 'scripts/verify/capture-env-fingerprint.ps1'
    host          = [ordered]@{
        label = $env:COMPUTERNAME
    }
    os = [ordered]@{
        caption             = $caption
        version             = '{0}.{1}.{2}' -f $osVersion.Major, $osVersion.Minor, $osVersion.Build
        buildNumber         = $buildNumber
        updateBuildRevision = $ubr
        fullBuild           = '{0}.{1}' -f $buildNumber, $ubr
        displayVersion      = $displayVersion
        editionId           = $editionId
        architecture        = $osArch
    }
    cpu = [ordered]@{
        name             = if ($cpuKey -and $cpuKey.PSObject.Properties['ProcessorNameString']) { [string]$cpuKey.ProcessorNameString } else { $null }
        physicalCores    = $physicalCores
        logicalCores     = $logicalCores
        maxClockSpeedMhz = $maxClockSpeedMhz
    }
    memory = [ordered]@{
        totalPhysicalBytes = $totalMemBytes
    }
    gpu       = $gpus
    audio     = $audio
    toolchain = $toolchain
}

# ---------------------------------------------------------------------------
# Emit
# ---------------------------------------------------------------------------

$json = $fingerprint | ConvertTo-Json -Depth 6

# Sanity: the emitted document must parse cleanly before we commit it to disk.
$null = $json | ConvertFrom-Json

# Absolute output path. The SessionState API is blocked under
# ConstrainedLanguage; the fallback resolves absolute paths as-is and
# relative ones against the caller's location (same semantics for the
# existing usage patterns).
try {
    $resolvedOut = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutPath)
} catch {
    if (Split-Path -Path $OutPath -IsAbsolute) {
        $resolvedOut = $OutPath
    } else {
        $resolvedOut = Join-Path (Get-Location -PSProvider FileSystem).ProviderPath $OutPath
    }
}
$outDir = Split-Path -Parent $resolvedOut
if (-not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}
Write-Utf8NoBomFile -Path $resolvedOut -Text ($json + [Environment]::NewLine)

if ($script:degradedSections.Count -gt 0) {
    Write-Host ("capture-env-fingerprint: WARNING: degraded sections: {0}" -f
        ($script:degradedSections -join '; ')) -ForegroundColor Yellow
}

Write-Host "Fingerprint written to $resolvedOut"
