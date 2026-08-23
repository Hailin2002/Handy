<#
.SYNOPSIS
    Runs an arbitrary command under a hard millisecond timeout, streaming its
    output to a log file and preserving whatever was produced before expiry.

.DESCRIPTION
    Starts <FilePath> with the given arguments as a child process, redirects
    stdout and stderr into the log file line-by-line while the child runs,
    and waits at most TimeoutMs milliseconds for it to finish. If the child
    is still running after that, the entire process tree is killed, whatever
    partial output was produced is flushed to the log together with a
    timeout marker, and the script exits with the dedicated timeout code
    124. If the child finishes in time, its own exit code is passed through
    unchanged. Every wait after the deadline (kill, reap, final drain)
    draws from one shared KillGraceMs budget, so the wrapper always
    terminates within roughly TimeoutMs + KillGraceMs plus the cost of a
    few process spawns, which is the only unbounded component and is
    operating-system dependent.

.PARAMETER FilePath
    Executable to run (mandatory, position 0). Resolved via PATH like any
    normal process start.

.PARAMETER ArgumentList
    Remaining positional parameters are forwarded verbatim as individual
    arguments to the executable. Arguments are re-quoted following the
    MSVCRT command-line rules, so values containing spaces survive.

.PARAMETER TimeoutMs
    Hard wall-clock budget for the child in milliseconds. Default: 120000.

.PARAMETER LogPath
    Path of the log file output is streamed into. The file is appended to
    and its parent directory is created if missing. Default: a unique file
    under the system temp directory. The resolved path is printed to the
    host at the end of every run.

.PARAMETER KillGraceMs
    Extra milliseconds granted to the child after the kill signal before
    the wrapper stops waiting on it. Default: 2000.

.EXAMPLE
    ./scripts/verify/run-with-timeout.ps1 bun run build -TimeoutMs 300000

.EXAMPLE
    ./scripts/verify/run-with-timeout.ps1 powershell -NoProfile -Command "Start-Sleep 60" -TimeoutMs 2000
    # exits 124 after ~2s; log contains any partial output plus the marker

.NOTES
    Exit code contract:
      <child exit code>  child finished within TimeoutMs (pass-through);
                         also reflected in $LASTEXITCODE for in-session
                         callers
      124                child was killed because it exceeded TimeoutMs
                         (GNU coreutils timeout convention)
      125                wrapper failed to start the child process
      126                invalid parameters (e.g. empty FilePath)
      2                  unsupported PowerShell language mode
                         (ConstrainedLanguage); see the compatibility note
                         below

    ConstrainedLanguage compatibility: parameter validation (exit 126) is
    written with only cmdlet/core-type constructs so it completes cleanly
    under ConstrainedLanguage. The core wrapper itself is NOT reducible to
    cmdlets: it requires System.Diagnostics.ProcessStartInfo/Process
    (child-process spawn), System.IO.StreamWriter (log streaming) and
    Add-Type P/Invoke (Windows Job Object tree containment), all of which
    the language mode blocks with "Cannot create type / Cannot invoke
    method". Rather than fail with that cryptic engine error mid-run, an
    early guard detects ConstrainedLanguage and exits 2 with an explicit
    message naming the blocking constructs.

    Output transport: stdout/stderr are consumed through
    StreamReader.ReadLineAsync() tasks that are pumped from the main poll
    loop. This keeps the log current while the child runs, survives an
    abrupt tree kill without losing buffered lines, and avoids the
    PowerShell engine-event queue, which is only serviced while the
    pipeline is idle and therefore cannot be polled from a busy script.
#>
[CmdletBinding()]
param(
    # Deliberately not Mandatory: a missing or empty value must fall through
    # to the explicit validation below so the caller gets the documented
    # usage code 126 instead of the engine's generic binding failure (1).
    [Parameter(Position = 0)]
    [string]$FilePath,

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$ArgumentList,

    [ValidateRange(1, [int]::MaxValue)]
    [int]$TimeoutMs = 120000,

    [string]$LogPath,

    [ValidateRange(0, [int]::MaxValue)]
    [int]$KillGraceMs = 2000
)

# Exit-code constants must be defined before parameter validation so the
# usage branch below can emit the documented code 126.
$exitCodeUnsupportedMode = 2
$exitCodeTimeout = 124
$exitCodeStartFailure = 125
$exitCodeUsage = 126

# Writes one line to stderr. [Console]::Error.WriteLine is blocked under
# ConstrainedLanguage ("Cannot invoke method"), so it degrades to the host
# stream instead of throwing a cryptic language-mode error.
function Write-ConsoleError {
    param([string]$Message)

    try {
        [Console]::Error.WriteLine($Message)
    }
    catch {
        Write-Host $Message -ForegroundColor Red
    }
}

# Parameter validation must run before $ErrorActionPreference = "Stop":
# Write-Error would otherwise terminate the script with exit 1 instead of
# the documented usage code 126.
if ([string]::IsNullOrWhiteSpace($FilePath)) {
    Write-ConsoleError "run-with-timeout: -FilePath must be a non-empty executable name or path."
    exit $exitCodeUsage
}

# ConstrainedLanguage guard: everything below needs .NET constructs the
# language mode blocks (ProcessStartInfo/Process spawn, StreamWriter log,
# Add-Type Job Object P/Invoke). Fail here, explicitly, instead of dying
# mid-run on a cryptic "Cannot create type" engine error. Exit code 2 is
# documented in the header notes and is distinct from every child/timeout
# code this wrapper can produce.
if ($ExecutionContext.SessionState.LanguageMode -eq 'ConstrainedLanguage') {
    Write-ConsoleError (
        "run-with-timeout: ConstrainedLanguage mode is not supported. " +
        "Blocking constructs: System.Diagnostics.ProcessStartInfo/" +
        "System.Diagnostics.Process (child-process spawn), System.IO.StreamWriter " +
        "(log streaming), Add-Type P/Invoke (Windows Job Object containment). " +
        "Re-run under FullLanguage mode.")
    exit $exitCodeUnsupportedMode
}

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $LogPath) {
    $LogPath = Join-Path ([System.IO.Path]::GetTempPath()) (
        "run-with-timeout-{0}.log" -f [guid]::NewGuid().ToString("N")
    )
}

$logDir = Split-Path -Parent $LogPath
if ($logDir -and -not (Test-Path -LiteralPath $logDir)) {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
}

# Quotes a single argument following the MSVCRT command-line rules used by
# CreateProcess: whitespace-containing or empty arguments are wrapped in
# double quotes, backslashes are doubled only when they precede a quote.
function ConvertTo-CrtArgument {
    param([string]$Value)

    if ($null -eq $Value) {
        return '""'
    }
    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') {
        return $Value
    }

    $builder = [System.Text.StringBuilder]::new()
    [void]$builder.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq "\") {
            $backslashes++
            continue
        }
        if ($character -eq '"') {
            [void]$builder.Append("\", $backslashes * 2 + 1)
            [void]$builder.Append('"')
        }
        else {
            [void]$builder.Append("\", $backslashes)
            [void]$builder.Append($character)
        }
        $backslashes = 0
    }
    [void]$builder.Append("\", $backslashes * 2)
    [void]$builder.Append('"')
    return $builder.ToString()
}

$argumentLine = ""
if ($ArgumentList) {
    $argumentLine = (@($ArgumentList) | Where-Object { $null -ne $_ } |
        ForEach-Object { ConvertTo-CrtArgument $_ }) -join " "
}

$startInfo = [System.Diagnostics.ProcessStartInfo]@{
    FileName               = $FilePath
    Arguments              = $argumentLine
    UseShellExecute        = $false
    RedirectStandardOutput = $true
    RedirectStandardError  = $true
    CreateNoWindow         = $true
}

$process = $null
$writer = $null

# --- Process-tree containment via a Windows Job Object ---
#
# Per-process kill APIs (Process.Kill, taskkill /T, WMI) all require
# PROCESS_TERMINATE rights on every descendant individually; hardened CI
# sandboxes routinely deny those for grandchildren, leaking orphans. A job
# object avoids that: the wrapper owns the job handle, so
# TerminateJobObject needs no per-process rights at all, and
# JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE additionally kills any survivor when
# the wrapper exits for ANY reason - including being force-killed itself.
# If Add-Type is unavailable (ConstrainedLanguage) or assignment fails,
# everything degrades gracefully to the legacy mechanisms below.
$script:jobHandle = [IntPtr]::Zero
try {
    if (-not ("JobObj" -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JobObj {
  [DllImport("kernel32.dll", SetLastError = true)]
  public static extern IntPtr CreateJobObject(IntPtr a, string lpName);
  [DllImport("kernel32.dll", SetLastError = true)]
  public static extern bool SetInformationJobObject(IntPtr h, int t, ref EXT i, int len);
  [DllImport("kernel32.dll", SetLastError = true)]
  public static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
  [DllImport("kernel32.dll", SetLastError = true)]
  public static extern bool TerminateJobObject(IntPtr job, uint exitCode);
  [DllImport("kernel32.dll", SetLastError = true)]
  public static extern bool CloseHandle(IntPtr h);
  [StructLayout(LayoutKind.Sequential)]
  public struct IO_COUNTERS {
    public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount;
    public ulong ReadTransferCount, WriteTransferCount, OtherTransferCount;
  }
  [StructLayout(LayoutKind.Sequential)]
  public struct BASIC {
    public long PerProcessUserTimeLimit, PerJobUserTimeLimit;
    public uint LimitFlags;
    public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize;
    public uint ActiveProcessLimit;
    public UIntPtr Affinity;
    public uint PriorityClass, SchedulingClass;
  }
  [StructLayout(LayoutKind.Sequential)]
  public struct EXT {
    public BASIC Basic;
    public IO_COUNTERS IoInfo;
    public UIntPtr ProcessMemoryLimit, JobMemoryLimit;
    public UIntPtr PeakProcessMemoryUsed, PeakJobMemoryUsed;
  }
}
'@ -ErrorAction Stop
    }
    $script:jobHandle = [JobObj]::CreateJobObject([IntPtr]::Zero, $null)
    if ($script:jobHandle -ne [IntPtr]::Zero) {
        # JobObjectExtendedLimitInformation (9); flag 0x2000 =
        # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
        $limit = New-Object JobObj+EXT
        $limit.Basic.LimitFlags = 0x2000
        $size = [System.Runtime.InteropServices.Marshal]::SizeOf([type][JobObj+EXT])
        [void][JobObj]::SetInformationJobObject($script:jobHandle, 9, [ref]$limit, $size)
    }
}
catch {
    $script:jobHandle = [IntPtr]::Zero
}

# Kills a process tree. Prefers the managed tree kill available on .NET Core
# (PowerShell 7+); then the wrapper-owned job object (TerminateJobObject
# needs no per-process rights and also covers descendants that joined the
# job); falls back to a self-bounded taskkill /T on Windows PowerShell 5.1
# where Process.Kill(entireProcessTree) does not exist, then to killing the
# root directly. Survivors are reaped by KILL_ON_JOB_CLOSE when the wrapper
# exits regardless. Never lets a failed kill mechanism abort the caller,
# and every internal wait is capped both by its own 0.5s ceiling and
# by the caller-supplied Deadline, whichever is sooner. taskkill is spawned
# through raw System.Diagnostics.Process rather than the Start-Process cmdlet
# because the cmdlet adds >1s of engine overhead on Windows PowerShell 5.1,
# which would blow the post-timeout grace budget.
function Stop-ProcessTree {
    param(
        [System.Diagnostics.Process]$Target,
        [datetime]$Deadline = [datetime]::MaxValue
    )

    if ($Target.HasExited) {
        return
    }
    # Managed tree kill exists only on .NET Core (PowerShell 7+). Probe for
    # the overload via reflection instead of try/catch: throwing and
    # swallowing a MethodException costs measurable time on Windows
    # PowerShell 5.1, inside the post-timeout grace budget.
    $treeKill = $Target.GetType().GetMethod("Kill", [Type[]]@([bool]))
    if ($treeKill) {
        try {
            $Target.Kill($true)
            return
        }
        catch {
        }
    }
    if ($script:jobHandle -ne [IntPtr]::Zero) {
        try {
            if ([JobObj]::TerminateJobObject($script:jobHandle, $exitCodeTimeout)) {
                return
            }
        }
        catch {
        }
    }
    $taskkill = Join-Path $env:SystemRoot "System32\taskkill.exe"
    if ($env:SystemRoot -and (Test-Path -LiteralPath $taskkill)) {
        try {
            $killerInfo = [System.Diagnostics.ProcessStartInfo]@{
                FileName               = $taskkill
                Arguments              = "/PID $($Target.Id) /T /F"
                UseShellExecute        = $false
                CreateNoWindow         = $true
            }
            $killer = [System.Diagnostics.Process]::Start($killerInfo)
            $waitMs = [Math]::Min(
                500,
                [int]($Deadline - [datetime]::UtcNow).TotalMilliseconds)
            if ($waitMs -gt 0) {
                [void]$killer.WaitForExit($waitMs)
            }
            if (-not $killer.HasExited) {
                try {
                    $killer.Kill()
                }
                catch {
                }
            }
            $killer.Dispose()
        }
        catch {
        }
    }
    # Last-resort root kill via the .NET API, not the Stop-Process cmdlet:
    # a cold cmdlet invocation costs far more than the method call and this
    # runs inside the post-timeout grace budget.
    try {
        if (-not $Target.HasExited) {
            $Target.Kill()
        }
    }
    catch {
    }
}

try {
    try {
        $process = [System.Diagnostics.Process]::Start($startInfo)
    }
    catch {
        Write-Host "run-with-timeout: failed to start '$FilePath': $($_.Exception.Message)" -ForegroundColor Red
        exit $exitCodeStartFailure
    }

    # Put the child into the wrapper-owned job object immediately. Without
    # this assignment TerminateJobObject would be a no-op and the "tree
    # kill" would degrade to root-only, orphaning grandchildren. Since
    # Windows 8 a process may belong to nested jobs, so assignment also
    # works when the harness already placed us in one. Failure is not
    # fatal: the fallback kill chain below still applies.
    if ($script:jobHandle -ne [IntPtr]::Zero) {
        try {
            [void][JobObj]::AssignProcessToJobObject($script:jobHandle, $process.Handle)
        }
        catch {
        }
    }

    $writer = [System.IO.StreamWriter]::new($LogPath, $true, [System.Text.Encoding]::UTF8)
    $writer.AutoFlush = $true
    # Millisecond-precision UTC timestamps so callers can verify the
    # timeout + grace budget directly from the log.
    $writer.WriteLine(
        "[run-with-timeout] started: {0:yyyy-MM-dd HH:mm:ss.fff}Z; command: $FilePath $argumentLine",
        [datetime]::UtcNow)

    # Async line readers: a completed task yields one line; a null result
    # marks end-of-stream (or a faulted read after the process died) and
    # retires that stream. Tasks complete on threadpool threads regardless
    # of what the PowerShell engine is doing, so pumping works mid-loop.
    $script:pump = @{
        OutReader = $process.StandardOutput
        OutTask   = $process.StandardOutput.ReadLineAsync()
        ErrReader = $process.StandardError
        ErrTask   = $process.StandardError.ReadLineAsync()
    }

    # Moves every completed line from the async readers into the log.
    function Read-PumpToLog {
        param([System.IO.StreamWriter]$Target)

        foreach ($side in @("Out", "Err")) {
            while ($true) {
                $task = $script:pump["${side}Task"]
                if ($null -eq $task -or -not $task.IsCompleted) {
                    break
                }
                $line = $null
                try {
                    $line = $task.Result
                }
                catch {
                }
                if ($null -eq $line) {
                    $script:pump["${side}Task"] = $null
                    break
                }
                $Target.WriteLine($line)
                $script:pump["${side}Task"] =
                    $script:pump["${side}Reader"].ReadLineAsync()
            }
        }
    }

    # Keeps pumping until both streams retired or the deadline passes.
    # Returns $true only when both streams reached end-of-stream.
    function Wait-PumpDrain {
        param([System.IO.StreamWriter]$Target, [datetime]$Deadline)

        while ([datetime]::UtcNow -lt $Deadline) {
            Read-PumpToLog -Target $Target
            if ($null -eq $script:pump["OutTask"] -and $null -eq $script:pump["ErrTask"]) {
                return $true
            }
            Start-Sleep -Milliseconds 25
        }
        Read-PumpToLog -Target $Target
        return $false
    }

    $deadline = [datetime]::UtcNow.AddMilliseconds($TimeoutMs)
    $timedOut = $false
    while ($true) {
        Read-PumpToLog -Target $writer
        if ($process.HasExited) {
            break
        }
        if ([datetime]::UtcNow -ge $deadline) {
            $timedOut = $true
            break
        }
        [void]$process.WaitForExit(50)
    }

    if ($timedOut) {
        Write-Host "run-with-timeout: timed out after ${TimeoutMs}ms; killing process tree of PID $($process.Id)" -ForegroundColor Yellow
        # One shared grace budget covers kill, reap, and the final drain so
        # total runtime stays within TimeoutMs + KillGraceMs (+ poll
        # granularity), no matter how many phases run.
        $reapDeadline = [datetime]::UtcNow.AddMilliseconds($KillGraceMs)
        Stop-ProcessTree -Target $process -Deadline $reapDeadline
        $remainingMs = [int]($reapDeadline - [datetime]::UtcNow).TotalMilliseconds
        if ($remainingMs -gt 0 -and -not $process.HasExited) {
            [void]$process.WaitForExit($remainingMs)
        }
        if (-not $process.HasExited) {
            try {
                $process.Kill()
            }
            catch {
            }
            $remainingMs = [int]($reapDeadline - [datetime]::UtcNow).TotalMilliseconds
            if ($remainingMs -gt 0) {
                [void]$process.WaitForExit($remainingMs)
            }
        }
    }
    else {
        # Parameterless WaitForExit lets the async readers observe EOF so
        # trailing lines reach their tasks before the final drain.
        $process.WaitForExit()
    }

    # Final bounded drain. When the process died its pipes close and both
    # streams retire quickly (EOF tasks are harvested instantly by the first
    # Read-PumpToLog); a stray grandchild inheriting the pipe handles cannot
    # stall the wrapper beyond this window. After a timeout the window is
    # whatever remains of the shared grace budget, keeping the overall
    # runtime inside TimeoutMs + KillGraceMs.
    $drainWindowMs = 500
    if ($timedOut) {
        $drainWindowMs = [int]($reapDeadline - [datetime]::UtcNow).TotalMilliseconds
    }
    if ($drainWindowMs -lt 0) {
        $drainWindowMs = 0
    }
    [void](Wait-PumpDrain -Target $writer -Deadline (
            [datetime]::UtcNow.AddMilliseconds($drainWindowMs)))

    if ($timedOut) {
        $writer.WriteLine(
            "[run-with-timeout] TIMED OUT at {0:yyyy-MM-dd HH:mm:ss.fff}Z after ${TimeoutMs}ms; process tree killed (PID {1})",
            [datetime]::UtcNow, $process.Id)
        $exitCode = $exitCodeTimeout
    }
    else {
        $exitCode = $process.ExitCode
    }

    $writer.Flush()
    Write-Host "run-with-timeout: exit code $exitCode; log: $LogPath"
    exit $exitCode
}
finally {
    if ($writer) {
        $writer.Dispose()
    }
    if ($process -and -not $process.HasExited) {
        Stop-ProcessTree -Target $process
    }
    if ($process) {
        $process.Dispose()
    }
}
