<#
.SYNOPSIS
    Clipboard/paste output verification with Unicode, CJK, RTL and large payloads.

.DESCRIPTION
    Drives the headless/CLI transcription *output* path contract on Windows:
    Handy writes transcription text to the system clipboard via
    tauri-plugin-clipboard-manager (arboard -> Win32 CF_UNICODETEXT, see
    src-tauri/src/clipboard.rs write_text_to_clipboard / paste_via_clipboard),
    then injects Ctrl+V into the target application. The target-side fidelity
    is verified here by scripted clipboard reads instead of a real paste
    window (isolated lane, no external side effects):

      1. Clipboard integrity: every fixture is written to the clipboard exactly
         like the app does (CF_UNICODETEXT, UTF-16LE, null-terminated) and read
         back through both the raw Win32 API and the .NET reader; content must
         match byte-for-byte (UTF-16LE and UTF-8 SHA-256).
      2. Paste-injection fidelity: the app's save/write/read/restore clipboard
         cycle (paste_via_clipboard) is replayed; the pre-paste clipboard must
         be restored byte-for-byte after the simulated paste.
      3. Large payload: >50 KB payload must survive without truncation; timing
         is recorded.

    Evidence (before/after dumps + SHA-256 hashes, timings, summary JSON) is
    written to the log directory. Exit code 0 = all gates pass.

.NOTES
    Read-only with respect to product code. Only writes its own log directory.

    Exit codes: 0 = all gates pass, 1 = at least one check failed,
    2 = unsupported PowerShell language mode (ConstrainedLanguage).

    ConstrainedLanguage compatibility: this probe is NOT reducible to
    cmdlets. Its core is Add-Type P/Invoke compilation of
    HandyVerify.NativeClipboard (user32/kernel32 clipboard APIs) and
    System.Security.Cryptography SHA256 hashing, both of which
    ConstrainedLanguage blocks with "Cannot add type / Cannot invoke
    method". An early guard detects ConstrainedLanguage and exits 2 with an
    explicit message naming the blocking constructs instead of a cryptic
    engine error mid-run. Two cheap constructs are nonetheless replaced so
    the script stays closer to CLM-clean: Stopwatch timing -> Get-Date /
    New-TimeSpan deltas, and [IO.File]::WriteAllText + UTF8Encoding::new ->
    Write-Utf8NoBomFile helper (Set-Content fallback; Windows PowerShell 5.1
    prepends a BOM under ConstrainedLanguage).
#>

[CmdletBinding()]
param(
    # Evidence directory (declared conflict scope for this leaf).
    # Resolved after the param block: $PSScriptRoot is empty in PS 5.1 param defaults.
    [string]$LogDir = '',
    # Minimum size in UTF-8 bytes the large-payload fixture must exceed (gate: >50 KB).
    [int]$MinLargeBytes = 51200,
    # Target size in UTF-8 bytes for the large payload fixture.
    [int]$LargeTargetBytes = 65536,
    # Repetitions of the full matrix (catches clipboard-lock flakiness).
    [int]$Repetitions = 2
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# ConstrainedLanguage guard: the clipboard fidelity probes require Add-Type
# P/Invoke (user32/kernel32 clipboard APIs) and .NET SHA256 hashing, both
# blocked by the language mode. Fail here, explicitly, instead of dying
# mid-run on a cryptic "Cannot add type" engine error. Exit code 2 is
# distinct from every check result this script can produce.
if ($ExecutionContext.SessionState.LanguageMode -eq 'ConstrainedLanguage') {
    Write-Host "test-clipboard-output: ConstrainedLanguage mode is not supported. Blocking constructs: Add-Type P/Invoke compilation (HandyVerify.NativeClipboard: user32/kernel32 clipboard APIs) and System.Security.Cryptography SHA256 hashing. Re-run under FullLanguage mode." -ForegroundColor Red
    exit 2
}

if ([string]::IsNullOrEmpty($LogDir)) {
    $LogDir = Join-Path $PSScriptRoot '..\..\src-tauri\target\baseline-tools\logs\clipboard-unicode'
}

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

function Write-Utf8NoBomFile {
    # UTF-8 without BOM writer. Primary path is byte-identical to the previous
    # direct [IO.File]::WriteAllText + UTF8Encoding($false) calls; under CLM
    # the .NET write is blocked and Set-Content keeps the run alive (Windows
    # PowerShell 5.1 Set-Content prepends a BOM - documented residual limit).
    param([string]$Path, [string]$Text)

    try {
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($Path, $Text, $utf8NoBom)
    }
    catch {
        Set-Content -LiteralPath $Path -Value $Text -Encoding UTF8
    }
}

# --- Win32 clipboard P/Invoke -------------------------------------------------
# Mirrors how arboard (used by tauri-plugin-clipboard-manager) talks to the
# Windows clipboard: OpenClipboard/SetClipboardData with CF_UNICODETEXT.
if (-not ('HandyVerify.NativeClipboard' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace HandyVerify
{
    public static class NativeClipboard
    {
        public const uint CF_UNICODETEXT = 13;
        private const uint GMEM_MOVEABLE = 0x0002;

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool OpenClipboard(IntPtr hWndNewOwner);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool CloseClipboard();

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool EmptyClipboard();

        [DllImport("user32.dll", SetLastError = true)]
        public static extern IntPtr GetClipboardData(uint uFormat);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern IntPtr SetClipboardData(uint uFormat, IntPtr hMem);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr GlobalAlloc(uint uFlags, UIntPtr dwBytes);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr GlobalLock(IntPtr hMem);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GlobalUnlock(IntPtr hMem);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern UIntPtr GlobalSize(IntPtr hMem);

        private static void WithOpenClipboard(Action action)
        {
            // The clipboard can transiently be owned by another process; retry.
            const int attempts = 20;
            for (int i = 0; i < attempts; i++)
            {
                if (OpenClipboard(IntPtr.Zero))
                {
                    try { action(); }
                    finally { if (!CloseClipboard()) throw new Exception("CloseClipboard failed"); }
                    return;
                }
                System.Threading.Thread.Sleep(25);
            }
            throw new Exception("OpenClipboard failed after " + attempts + " attempts (error " + Marshal.GetLastWin32Error() + ")");
        }

        /// Writes text as CF_UNICODETEXT (UTF-16LE, null terminated), like arboard does.
        public static void SetText(string text)
        {
            byte[] bytes = System.Text.Encoding.Unicode.GetBytes(text);
            WithOpenClipboard(() =>
            {
                EmptyClipboard();
                IntPtr hMem = GlobalAlloc(GMEM_MOVEABLE, (UIntPtr)(bytes.Length + 2));
                if (hMem == IntPtr.Zero) throw new Exception("GlobalAlloc failed");
                IntPtr ptr = GlobalLock(hMem);
                if (ptr == IntPtr.Zero) { Marshal.FreeHGlobal(hMem); throw new Exception("GlobalLock failed"); }
                try
                {
                    Marshal.Copy(bytes, 0, ptr, bytes.Length);
                    Marshal.WriteInt16(ptr, bytes.Length, 0); // null terminator (2 bytes)
                }
                finally { GlobalUnlock(hMem); }
                IntPtr result = SetClipboardData(CF_UNICODETEXT, hMem);
                if (result == IntPtr.Zero)
                    throw new Exception("SetClipboardData failed (error " + Marshal.GetLastWin32Error() + ")");
                // On success the system owns hMem; do not free it.
            });
        }

        /// Reads CF_UNICODETEXT raw UTF-16LE bytes (excluding the null terminator).
        public static byte[] GetTextRawBytes()
        {
            byte[] result = Array.Empty<byte>();
            WithOpenClipboard(() =>
            {
                if (!IsClipboardFormatAvailableNative(CF_UNICODETEXT)) return;
                IntPtr handle = GetClipboardData(CF_UNICODETEXT);
                if (handle == IntPtr.Zero) throw new Exception("GetClipboardData returned NULL");
                UIntPtr sizePtr = GlobalSize(handle);
                int size = unchecked((int)(long)sizePtr);
                if (size <= 0) throw new Exception("GlobalSize returned 0");
                IntPtr ptr = GlobalLock(handle);
                if (ptr == IntPtr.Zero) throw new Exception("GlobalLock failed on read");
                try
                {
                    byte[] raw = new byte[size];
                    Marshal.Copy(ptr, raw, 0, size);
                    // Decode UTF-16LE and cut at the first null character the OS
                    // appends as terminator (embedded content cannot contain one).
                    int charCount = size / 2;
                    int end = charCount;
                    for (int i = 0; i < charCount; i++)
                    {
                        if (raw[2 * i] == 0 && raw[2 * i + 1] == 0) { end = i; break; }
                    }
                    result = new byte[end * 2];
                    Array.Copy(raw, result, result.Length);
                }
                finally { GlobalUnlock(handle); }
            });
            return result;
        }

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool IsClipboardFormatAvailable(uint format);

        // Classic method body: the in-box PS 5.1 compiler only understands C# 5,
        // so expression-bodied members are not available here.
        private static bool IsClipboardFormatAvailableNative(uint format)
        {
            return IsClipboardFormatAvailable(format);
        }
    }
}
'@
}

# --- Helpers -------------------------------------------------------------------
function Get-Sha256([byte[]]$Bytes) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { ([System.BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Get-Utf8Bytes([string]$Text) { [System.Text.Encoding]::UTF8.GetBytes($Text) }

# Raw CF_UNICODETEXT bytes decoded back to string (authoritative reader).
function Read-ClipboardRawString {
    $bytes = [HandyVerify.NativeClipboard]::GetTextRawBytes()
    if ($null -eq $bytes) { return $null }
    return [System.Text.Encoding]::Unicode.GetString($bytes)
}

# --- Fixtures ------------------------------------------------------------------
$fixtures = [ordered]@{
    # German low/high typographic quotes are emitted via [char] subexpressions:
    # a literal „ or “ in source is parsed by PowerShell as a string delimiter
    # and breaks compilation.
    'de-umlauts' = ("Grüße aus Köln! Die größte Überraschung: Ärger mit süßem Käse.`r`n" +
                    "ÄÖÜ äöü ß Ÿ Œ œ – $([char]0x201E)deutsche Anführungszeichen$([char]0x201C) · «französische» …")
    'emoji'      = ("Hallo 👋 Welt 🌍 — ZWJ-Familie: 👨‍👩‍👧‍👦 Hautton: 👍🏽🤝🏿 Flagge: 🇩🇪🇯🇵 " +
                    "Keycap: 1️⃣ #️⃣ Variation: ✔️ ☺️ Kombinierend: é́ê CJK-Emoji: 🈶㊗️ " +
                    "Surrogates: 𝕳𝖆𝖓𝖉𝖞 🔡➡️🔤")
    'cjk'        = ("简体中文测试：语音转文字系统。" + "`n" +
                    "繁體中文測試：語音轉文字系統。" + "`n" +
                    "日本語のテスト：音声認識システム、ひらがな・カタカナ・漢字。" + "`n" +
                    "한국어 테스트: 음성 인식 시스템, 한글 자모 ㅎㅏㄴㄷㅣ.")
    'rtl'        = ("مرحبا بالعالم، هذا اختبار للنص العربي مع أرقام ١٢٣ وأرقام لاتينية 456." + "`n" +
                    "זהו מבדק עברי עם מספרים 789 ותווים מעורבים." + "`n" +
                    "Mixed bidi: ‏RTL run inside LTR‏ and ‪LTR run inside RTL‬ end.")
    'combined'   = $null # assembled below from all classes
}
$fixtures['combined'] = (
    ($fixtures['de-umlauts'], $fixtures['emoji'], $fixtures['cjk'], $fixtures['rtl']) -join "`r`n---`r`n"
) + "`r`nTabs:`tTab`tEnd`vVerticalSep"

# Deterministic large payload (>50 KB gate): repeated multilingual paragraph.
$unit = (($fixtures['de-umlauts'] + "`r`n"), ($fixtures['cjk'] + "`r`n"),
         ($fixtures['emoji'] + "`r`n"), ($fixtures['rtl'] + "`r`n")) -join ''
$sb = [System.Text.StringBuilder]::new()
while ([System.Text.Encoding]::UTF8.GetByteCount($sb.ToString()) -lt $LargeTargetBytes) {
    [void]$sb.Append($unit); [void]$sb.Append("[chunk $(([System.Text.Encoding]::UTF8.GetByteCount($sb.ToString()))) marker ✓]`r`n")
}
$fixtures['large-payload'] = $sb.ToString()

# --- Baseline: capture user's current clipboard (before dump + hash) ------------
$baselineRaw = [HandyVerify.NativeClipboard]::GetTextRawBytes()
$baselineText = if ($null -ne $baselineRaw) { [System.Text.Encoding]::Unicode.GetString($baselineRaw) } else { '' }
$baselineFile = Join-Path $LogDir 'before-clipboard-dump.txt'
Write-Utf8NoBomFile -Path $baselineFile -Text $baselineText
$baselineHash = Get-Sha256 $(Get-Utf8Bytes $baselineText)
Write-Output "Baseline clipboard: $($baselineRaw.Count) bytes, sha256=$baselineHash" |
    Tee-Object -FilePath (Join-Path $LogDir 'transcript.log') -Append

try {

$results = [System.Collections.Generic.List[object]]::new()
$failures = 0

foreach ($rep in 1..$Repetitions) {
    foreach ($name in $fixtures.Keys) {
        $text = $fixtures[$name]
        $utf8Expected = Get-Utf8Bytes $text
        $utf16Expected = [System.Text.Encoding]::Unicode.GetBytes($text)

        # Gate check for the large payload class itself.
        if ($name -eq 'large-payload' -and $utf8Expected.Length -le $MinLargeBytes) {
            Write-Host "FAIL [$rep/$name]: fixture only $($utf8Expected.Length) bytes, must exceed $MinLargeBytes"
            $failures++; continue
        }

        # -- simulate paste_via_clipboard: save prior content --------------------
        $savedRaw = [HandyVerify.NativeClipboard]::GetTextRawBytes()
        $savedText = if ($null -ne $savedRaw) { [System.Text.Encoding]::Unicode.GetString($savedRaw) } else { $null }

        # -- write like write_text_to_clipboard ----------------------------------
        # Get-Date delta instead of System.Diagnostics.Stopwatch (CLM-blocked);
        # same millisecond precision.
        $writeStartedAt = Get-Date
        [HandyVerify.NativeClipboard]::SetText($text)
        $writeMs = (New-TimeSpan -Start $writeStartedAt -End (Get-Date)).TotalMilliseconds

        # -- scripted clipboard read (paste-injection fidelity probe) ------------
        $readStartedAt = Get-Date
        $rawBack = [HandyVerify.NativeClipboard]::GetTextRawBytes()
        $rawStr = if ($null -ne $rawBack) { [System.Text.Encoding]::Unicode.GetString($rawBack) } else { $null }
        $dotNetRead = Get-Clipboard -Raw
        $readMs = (New-TimeSpan -Start $readStartedAt -End (Get-Date)).TotalMilliseconds

        # -- byte-for-byte comparisons -------------------------------------------
        $utf16Match = ($null -ne $rawBack) -and ($rawBack.Length -eq $utf16Expected.Length) -and
                      ([System.Linq.Enumerable]::SequenceEqual($rawBack, $utf16Expected))
        $utf8HashMatch = ($null -ne $rawStr) -and ((Get-Sha256 (Get-Utf8Bytes $rawStr)) -eq (Get-Sha256 $utf8Expected))
        $dotNetMatch = ($null -ne $dotNetRead) -and ($dotNetRead -ceq $text)

        # -- restore prior clipboard like finish_clipboard_paste ------------------
        if ($null -ne $savedText) { [HandyVerify.NativeClipboard]::SetText($savedText) } else { Set-Clipboard -Value ' ' }
        Start-Sleep -Milliseconds 30
        $restoredRaw = [HandyVerify.NativeClipboard]::GetTextRawBytes()
        $restoredText = if ($null -ne $restoredRaw) { [System.Text.Encoding]::Unicode.GetString($restoredRaw) } else { $null }
        $restoreOk = if ($null -eq $savedText) { $true } else { ($null -ne $restoredText) -and ($restoredText -ceq $savedText) }

        $ok = $utf16Match -and $utf8HashMatch -and $dotNetMatch -and $restoreOk
        if (-not $ok) { $failures++ }

        # -- evidence: dumps + hashes --------------------------------------------
        $prefix = "{0}-rep{1}" -f $name, $rep
        $beforePath = Join-Path $LogDir "$prefix-before.txt"
        $afterPath  = Join-Path $LogDir "$prefix-after.txt"
        Write-Utf8NoBomFile -Path $beforePath -Text $text
        if ($null -ne $rawStr) { Write-Utf8NoBomFile -Path $afterPath -Text $rawStr }

        $results.Add([pscustomobject]@{
            repetition       = $rep
            fixture          = $name
            utf8_bytes       = $utf8Expected.Length
            utf16_bytes      = $utf16Expected.Length
            utf16_bytewise_equal = [bool]$utf16Match
            utf8_sha256_expected = Get-Sha256 $utf8Expected
            utf8_sha256_readback = if ($null -ne $rawStr) { Get-Sha256 (Get-Utf8Bytes $rawStr) } else { $null }
            utf8_hash_equal  = [bool]$utf8HashMatch
            dotnet_reader_equal = [bool]$dotNetMatch
            clipboard_restore_ok = [bool]$restoreOk
            write_ms         = [math]::Round($writeMs, 2)
            read_ms          = [math]::Round($readMs, 2)
            pass             = [bool]$ok
        })

        Write-Output ("{0} rep{1} {2,-14} {3,7} B  utf16={4} hash={5} net={6} restore={7} w={8}ms r={9}ms" -f `
            ($(if ($ok) {'PASS'} else {'FAIL'})), $rep, $name, $utf8Expected.Length,
             $utf16Match, $utf8HashMatch, $dotNetMatch, $restoreOk,
             [math]::Round($writeMs,1), [math]::Round($readMs,1))
    }
}

# --- Final: verify user clipboard restored to baseline (after dump + hash) ------
if ($baselineRaw.Count -gt 0) { [HandyVerify.NativeClipboard]::SetText($baselineText) }
Start-Sleep -Milliseconds 50
$finalRaw = [HandyVerify.NativeClipboard]::GetTextRawBytes()
$finalText = if ($null -ne $finalRaw) { [System.Text.Encoding]::Unicode.GetString($finalRaw) } else { '' }
$finalFile = Join-Path $LogDir 'after-clipboard-dump.txt'
Write-Utf8NoBomFile -Path $finalFile -Text $finalText
$finalHash = Get-Sha256 $(Get-Utf8Bytes $finalText)
$baselinePreserved = ($finalHash -eq $baselineHash)
Write-Output "Final clipboard: sha256=$finalHash baseline-preserved=$baselinePreserved"

$largeRows = @($results | Where-Object { $_.fixture -eq 'large-payload' })
$summary = [pscustomobject]@{
    timestamp_utc     = [DateTime]::UtcNow.ToString('o')
    platform          = "Windows $($env:COMPUTERNAME) PS $($PSVersionTable.PSVersion)"
    repetitions       = $Repetitions
    gates = @{
        g1_all_fixture_classes_roundtrip = [bool]($results.Count -gt 0 -and
                                                  -not ($results | Where-Object { $_.fixture -ne 'large-payload' -and -not $_.pass }))
        g2_large_payload_no_truncation   = [bool](($largeRows | Where-Object { $_.utf16_bytewise_equal }).Count -eq $largeRows.Count) -and
                                            $largeRows[0].utf8_bytes -gt $MinLargeBytes
        g3_before_after_dumps_hashed     = (Test-Path $baselineFile) -and (Test-Path $finalFile) -and
                                            ($baselineHash.Length -eq 64) -and ($finalHash.Length -eq 64)
    }
    large_payload_utf8_bytes = $largeRows[0].utf8_bytes
    large_payload_write_ms   = $largeRows.write_ms
    large_payload_read_ms    = $largeRows.read_ms
    baseline_sha256_before   = $baselineHash
    baseline_sha256_after    = $finalHash
    baseline_preserved       = $baselinePreserved
    total_checks             = $results.Count
    failures                 = $failures + $(if (-not $baselinePreserved) { 1 } else { 0 })
    results                  = $results
}
$summary | ConvertTo-Json -Depth 5 | Set-Content -Path (Join-Path $LogDir 'summary.json') -Encoding utf8
$summary.results | Export-Csv -Path (Join-Path $LogDir 'results.csv') -NoTypeInformation -Encoding utf8
"$baselineHash  before-clipboard-dump.txt`n$finalHash  after-clipboard-dump.txt" |
    Set-Content (Join-Path $LogDir 'dumps.sha256') -Encoding ascii

Write-Output "G1=$($summary.gates.g1_all_fixture_classes_roundtrip) G2=$($summary.gates.g2_large_payload_no_truncation) G3=$($summary.gates.g3_before_after_dumps_hashed)"

if ($summary.failures -gt 0) { exit 1 } else { exit 0 }

} finally {
    # Best-effort restore of the user's clipboard even on unexpected failure.
    try { if ($baselineRaw.Count -gt 0) { [HandyVerify.NativeClipboard]::SetText($baselineText) } } catch {}
}
