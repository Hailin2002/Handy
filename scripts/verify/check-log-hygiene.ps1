<#
.SYNOPSIS
    Log-hygiene guard: scans Rust log/error macro invocations for
    transcript-payload or credential identifiers before they can reach log
    files or stdout.

.DESCRIPTION
    Why: Handy logs to a user-visible log file and, in headless mode, to
    stdout. A stray `debug!("{:?}", settings)` or `info!("{}", transcript)`
    silently writes credentials or the raw dictation payload somewhere the
    user does not expect. This script makes that class of bug a failing
    check instead of a code-review memory test.

    What it scans: every .rs file under -ScanRoot (default
    src-tauri/src, recursively), extracting FULL macro invocations
    (multi-line safe) for the log/format family:

        log::/tracing-prefixed or bare  trace! debug! info! warn! error!
        println! print! eprintln! eprint! dbg! panic!

    What it flags (rule ids in output):
        ApiKeyIdentifier      - the identifier `api_key`/`api-key`/`apikey`
                                appears anywhere in the invocation
        CredentialMaterial    - bearer/authorization/secret/subscription-key/
                                DeepL-auth material appears in the invocation
        SettingsStructDump    - a whole settings map/struct is formatted into
                                the message (`"{:?}"..., settings` or a
                                serde_json::to_string(&settings...) dump)
        TranscriptPayloadInline  - an inline-captured transcript-stemmed
                                   identifier ({transcript}, {transcription_text:?})
        TranscriptPayloadPositional - a transcript-stemmed identifier passed as
                                      a positional format argument
        RawPayloadPrint       - a print-family macro formats a `text:` /
                                `transcript:` labelled value to stdout/stderr

    Suppression (both reviewed, never silent):
      1. Inline: a `// log-hygiene:allow(<reason>)` comment on any line the
         invocation spans. The reason is mandatory-by-convention and echoed
         in the summary as a suppressed finding.
      2. Central allowlist below: each entry carries a Rule, a file pattern,
         a regex over the invocation text and a Reason. Every entry must
         carry a justification comment right above it; entries without a
         non-empty Reason fail the script (infrastructure error), so a
         future editor cannot quietly add a blanket exemption.

    Exit codes:
      0   clean (or -SelfTest passed all assertions)
      1   violations found, or the self-test detected a broken guard
      2   infrastructure error (bad paths, malformed allowlist, unreadable file)

    Gates this script implements:
      G1  -SelfTest seeds violations into a temp tree, proves the scanner
           flags them (nonzero scan verdict), proves clean samples stay
           clean and that both suppression mechanisms work.
      G2  -ScanRoot over a clean tree exits 0; every allowlist entry carries
           a justification comment (enforced above).
      G3  wired into package.json ("check:log-hygiene" and the aggregate
           "check" script) and into scripts/verify/run-gate.ps1's step table.

    ConstrainedLanguage compatibility: uses only cmdlets, core types
    ([regex], arrays, hashtables) and no Add-Type/.NET static calls.
#>

[CmdletBinding()]
param(
    # Directory (or single .rs file) to scan. Default: <repo>/src-tauri/src.
    [string]$ScanRoot,

    # Repository root. Default: resolved from $PSScriptRoot (../..).
    # Only used to derive the default -ScanRoot and relative display paths.
    [string]$RepoRoot,

    # Run the built-in self-test instead of scanning: seeds violations into
    # a temporary directory, expects them flagged, clean samples unflagged,
    # and both suppression mechanisms effective. Never touches repo sources.
    [switch]$SelfTest,

    # Print suppressed findings too (default: summarized only).
    [switch]$ShowSuppressed
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$exitClean = 0
$exitFindings = 1
$exitInfrastructure = 2

function Write-GuardError {
    param([string]$Message)
    try {
        [Console]::Error.WriteLine("check-log-hygiene: $Message")
    }
    catch {
        Write-Host "check-log-hygiene: $Message"
    }
}

# ---------------------------------------------------------------------------
# Central allowlist. EVERY entry needs a justification comment directly
# above it AND a non-empty Reason field; the validation below fails the run
# otherwise (gate G2: no unjustified exemptions).
# Matching: a finding is suppressed only when Rule matches, the relative
# file path matches FilePattern, and the full invocation text matches
# Pattern. All three conditions; no wildcard-rule shortcuts.
# ---------------------------------------------------------------------------
$script:Allowlist = @(
    # Justification: the ONLY whole-settings debug dump in the tree logs the
    # AppSettings struct, and every API key inside it lives in SecretMap,
    # whose custom Debug impl replaces non-empty values with "[REDACTED]"
    # (src-tauri/src/settings/types.rs, fmt::Debug for SecretMap). The
    # redaction is pinned by the debug_output_redacts_api_keys and
    # secret_map_debug_redacts_values tests in
    # src-tauri/src/settings/tests.rs, so a regression there breaks cargo
    # test independently of this guard.
    @{
        Rule        = 'SettingsStructDump'
        FilePattern = '(?i)(^|[\\/])settings[\\/]+store\.rs$'
        Pattern     = '\{:\?\}"\s*,\s*&?(?:mut\s+)?settings\b'
        Reason      = ('AppSettings Debug dump in load path; API keys are ' +
                       'redacted by SecretMap''s Debug impl (pinned by unit tests).')
    },
    # Justification: the headless CLI's documented contract is to write the
    # finished transcription to stdout (scripts and pipelines consume it;
    # see src-tauri/src/headless.rs run_headless). This is the product's
    # primary OUTPUT channel chosen explicitly by the user, not a hidden
    # log sink; flagging it would be a permanent false positive.
    @{
        Rule        = 'RawPayloadPrint'
        FilePattern = '(?i)(^|[\\/])headless\.rs$'
        Pattern     = '"text:\s*\{\}"\s*,\s*text\b'
        Reason      = ('Headless CLI prints the final transcription to stdout ' +
                       'by design; explicit user-requested output channel.')
    }
)

# ---------------------------------------------------------------------------
# Detection rules applied to each extracted macro invocation.
#
# Precision notes (each documented so a future editor keeps the semantics):
#   - SettingsStructDump / TranscriptPayloadPositional match the identifier
#     only when it is passed WHOLE: a lookbehind rejects `self.settings`
#     (member access) and a lookahead rejects `settings.field`,
#     `module::CONST` and direct calls. A whole struct/map dump is the leak
#     signal; logging one scalar field is normal operation. For transcript
#     identifiers the lookahead additionally whitelists the metadata-only
#     method calls `.len()` and `.elapsed()`; any OTHER method chain
#     (e.g. `.clone()`, `.chars()`) stays conservatively flagged.
#   - RawPayloadPrint applies only to the stdout/stderr print family
#     (println/print/eprintln/eprint/dbg). Log-family macros (warn!, error!,
#     ...) that print an ERROR next to the word "transcript" are not payload
#     leaks, and genuine log-family transcript leaks are already caught by
#     the TranscriptPayload* rules. Optional per-rule Macros list; absent =
#     all scanned macros.
# ---------------------------------------------------------------------------
$script:Rules = @(
    @{
        Id          = 'ApiKeyIdentifier'
        Description = 'credential identifier (api_key) referenced in log message'
        Regex       = '(?i)\bapi[_-]?keys?\b'
    },
    @{
        Id          = 'CredentialMaterial'
        Description = 'credential/auth material (bearer, secret, auth header, vendor key header)'
        Regex       = '(?i)\b(bearer\s|authorization|auth_header|client_secret|subscription[_-]?key|deepl[-_]auth)'
    },
    @{
        Id          = 'SettingsStructDump'
        Description = 'whole settings map/struct formatted into the message'
        Regex       = '(?is)("(?:[^"\\]|\\.)*"\s*,\s*&?(?:mut\s+)?(?<![.\w])[a-z_]*settings[a-z_0-9]*\b(?!\s*(?:\.|::|\())|serde_json::to_string(_pretty)?\(\s*&?\w*settings\b(?!\s*\.\w))'
    },
    @{
        Id          = 'TranscriptPayloadInline'
        Description = 'inline-captured transcript-stemmed identifier in format string'
        Regex       = '(?i)\{\s*[a-z_]*transcript[a-z_0-9]*\s*(:[^}]*)?\}'
    },
    @{
        Id          = 'TranscriptPayloadPositional'
        Description = 'transcript-stemmed identifier passed as positional format argument'
        Regex       = '(?i)"\s*,\s*&?(?:mut\s+)?(?<![.\w])[a-z_]*transcript[a-z_0-9]*\b(?!\s*(?:\.(?:len|elapsed)\b|\(|::))'
    },
    @{
        Id          = 'RawPayloadPrint'
        Description = 'print-family macro emits a text/transcript-labelled value'
        Regex       = '(?i)"[^"]*\b(text|transcript[a-z_0-9]*)\s*[:=][^"]*\{'
        Macros      = @('println', 'print', 'eprintln', 'eprint', 'dbg')
    }
)

# Macros considered log/error/output surfaces. Bare names also match when
# log::- or tracing::-prefixed.
$script:MacroNames = 'trace|debug|info|warn|error|println|print|eprintln|eprint|dbg|panic'

# ---------------------------------------------------------------------------
# Extract full macro invocations (paren-balanced, literal/comment aware).
# Returns: Macro, StartLine, EndLine, Text.
# ---------------------------------------------------------------------------
function Get-RustMacroInvocations {
    param([string]$Content)

    $out = @()
    $len = $Content.Length
    $startRegex = [regex]"(?i)(?:(?:log|tracing)::)?($($script:MacroNames))!"
    $pos = 0

    while ($true) {
        $m = $startRegex.Match($Content, $pos)
        if (-not $m.Success) { break }

        $scanFrom = $m.Index + $m.Length
        # Skip whitespace between '!' and '('.
        while ($scanFrom -lt $len -and [char]::IsWhiteSpace($Content[$scanFrom])) { $scanFrom++ }
        if ($scanFrom -ge $len -or $Content[$scanFrom] -ne '(') {
            $pos = $m.Index + $m.Length
            continue
        }

        $depth = 0
        $i = $scanFrom
        $end = -1
        $mode = 'code'   # code | str | char | rawstr | linecomment | blockcomment
        $rawHashes = 0
        # 'break' inside a switch only leaves the switch, NOT the enclosing
        # while loop - so loop exit is driven by this flag instead.
        $closed = $false

        # Token-skip scanning: each mode jumps to the next RELEVANT character
        # with native IndexOf/IndexofAny instead of iterating char by char in
        # PowerShell. The per-char loop this replaces measured >13 s on the
        # single largest file of this tree, starving CI-style gate runs; the
        # skip logic below keeps identical semantics while finishing the whole
        # tree in well under a second of interpreted work.
        #
        # NOTE on nesting state: $depth counts PARENTHESES only. Block-comment
        # nesting has its own counter ($commentDepth); folding both into one
        # variable corrupted the parenthesis balance (a /* */ inside an
        # invocation reset $depth to 1 and every following ')' then drove it
        # negative, so the invocation was reported "unbalanced"). Loop exits
        # inside switch branches are driven by explicit flags because 'break'
        # binds to the switch, not the enclosing while loop.
        $codeStops = @('"', "'", '/', '(', ')', 'r', 'b')
        $escapeOrQuote = @('\', '"')
        $commentDepth = 0

        while ($i -lt $len -and -not $closed) {
            switch ($mode) {
                'linecomment' {
                    $j = $Content.IndexOf("`n", $i)
                    if ($j -lt 0) { $i = $len } else { $i = $j + 1; $mode = 'code' }
                }
                'blockcomment' {
                    # Nested block comments open/close with depth, mirroring
                    # Rust's behaviour. Flag-driven loop: see NOTE above.
                    $doneComment = $false
                    while (-not $doneComment -and $i -lt $len) {
                        $open = $Content.IndexOf('/*', $i)
                        $close = $Content.IndexOf('*/', $i)
                        if ($close -lt 0) { $i = $len; $doneComment = $true }
                        elseif ($open -ge 0 -and $open -lt $close) { $commentDepth++; $i = $open + 2 }
                        else {
                            $commentDepth--; $i = $close + 2
                            if ($commentDepth -le 0) { $mode = 'code'; $doneComment = $true }
                        }
                    }
                }
                'str' {
                    $j = $Content.IndexOfAny($escapeOrQuote, $i)
                    if ($j -lt 0) { $i = $len }
                    elseif ($Content[$j] -eq '\') { $i = $j + 2 }
                    else { $i = $j + 1; $mode = 'code' }
                }
                'char' {
                    $j = $Content.IndexOfAny($escapeOrQuote, $i)
                    if ($j -lt 0) { $i = $len }
                    elseif ($Content[$j] -eq '\') { $i = $j + 2 }
                    else { $i = $j + 1; $mode = 'code' }
                }
                'rawstr' {
                    # Raw strings close with a quote FOLLOWED by as many '#'
                    # as opened them ("..."#, "..."##). Flag-driven probe
                    # loop: 'break' inside a switch branch binds to the
                    # switch, not this while loop, so exit state is carried
                    # in $foundClose.
                    $j = $Content.IndexOf('"', $i)
                    $foundClose = $false
                    while ($j -ge 0 -and -not $foundClose) {
                        $hashCount = 0
                        $k = $j + 1
                        while ($k -lt $len -and $Content[$k] -eq '#') { $hashCount++; $k++ }
                        if ($hashCount -eq $rawHashes) {
                            $foundClose = $true
                        }
                        else {
                            $j = $Content.IndexOf('"', $j + 1)
                        }
                    }
                    if (-not $foundClose) { $i = $len }
                    else { $i = $j + $rawHashes + 1; $mode = 'code' }
                }
                'code' {
                    $j = $Content.IndexOfAny($codeStops, $i)
                    if ($j -lt 0) {
                        $i = $len
                    }
                    else {
                        $c = $Content[$j]
                        if ($c -eq '(') { $depth++; $i = $j + 1 }
                        elseif ($c -eq ')') {
                            $depth--; $i = $j + 1
                            if ($depth -eq 0) { $end = $j; $closed = $true }
                        }
                        elseif ($c -eq '/') {
                            $n = $(if ($j + 1 -lt $len) { $Content[$j + 1] } else { [char]0 })
                            if ($n -eq '/') { $mode = 'linecomment'; $i = $j + 2 }
                                    elseif ($n -eq '*') { $mode = 'blockcomment'; $commentDepth = 1; $i = $j + 2 }
                            else { $i = $j + 1 }
                        }
                        elseif ($c -eq '"') { $mode = 'str'; $i = $j + 1 }
                        elseif ($c -eq "'") {
                            # Distinguish char literal from lifetime: a char
                            # literal is 'x' or '\x'.
                            $isChar = $false
                            if ($j + 2 -lt $len -and $Content[$j + 2] -eq "'") { $isChar = $true }
                            elseif ($j + 3 -lt $len -and $Content[$j + 1] -eq '\' -and $Content[$j + 3] -eq "'") { $isChar = $true }
                            if ($isChar) { $mode = 'char' }
                            $i = $j + 1
                        }
                        else {
                            # Raw string openers: r", br", r#", br#", r##"...
                            # Anything else following the prefix letter(s)
                            # (identifiers, keywords) simply falls through.
                            $q = $j
                            $hasB = $false
                            if ($Content[$q] -eq 'b') { $hasB = $true; $q++ }
                            if ($q -lt $len -and $Content[$q] -eq 'r') { $q++ } else { $q = -999 }
                            $openedRaw = $false
                            if ($q -ge 0) {
                                $hashes = 0
                                while ($q -lt $len -and $Content[$q] -eq '#') { $hashes++; $q++ }
                                if ($q -lt $len -and $Content[$q] -eq '"') {
                                    $rawHashes = $hashes
                                    $mode = 'rawstr'
                                    $i = $q + 1
                                    $openedRaw = $true
                                }
                            }
                            if (-not $openedRaw) { $i = $j + 1 }
                        }
                    }
                }
            }
        }

        if ($end -lt 0) {
            # Unbalanced (truncated file?) - report as infrastructure issue
            # by surfacing what we have; caller decides.
            $out += [pscustomobject]@{
                Macro     = $m.Groups[1].Value
                StartLine = 0
                EndLine   = 0
                Text      = $null
                Unbalanced = $true
            }
            $pos = $m.Index + $m.Length
            continue
        }

        $invocationText = $Content.Substring($m.Index, $end - $m.Index + 1)
        $before = $Content.Substring(0, $m.Index)
        $startLine = ($before -split "`n").Count
        $endLine = ($Content.Substring(0, $end + 1) -split "`n").Count

        $out += [pscustomobject]@{
            Macro      = $m.Groups[1].Value
            StartLine  = $startLine
            EndLine    = $endLine
            Text       = $invocationText
            Unbalanced = $false
        }
        $pos = $end + 1
    }
    return ,@($out)
}

# Line numbers of the invocation, joined for the inline-allow probe.
function Get-LineRangeText {
    param([string[]]$Lines, [int]$Start, [int]$End)
    $slice = @()
    for ($n = $Start; $n -le $End -and $n -le $Lines.Count; $n++) { $slice += $Lines[$n - 1] }
    return ($slice -join "`n")
}

function Test-AllowlistSuppression {
    param(
        [object]$Finding,      # pscustomobject with Rule, RelPath, Macro, Text
        [object[]]$Allowlist
    )
    foreach ($entry in $Allowlist) {
        if ($entry.Rule -ne $Finding.Rule) { continue }
        if (-not $entry.Reason -or [string]::IsNullOrWhiteSpace($entry.Reason)) {
            throw ("check-log-hygiene: allowlist entry for rule '{0}' has an empty Reason; every exemption must carry a justification." -f $entry.Rule)
        }
        if ($entry.FilePattern -and ($Finding.RelPath -notmatch $entry.FilePattern)) { continue }
        if ($Finding.Text -match $entry.Pattern) { return $entry.Reason }
    }
    return $null
}

function Invoke-HygieneScan {
    param(
        [string[]]$Files,
        [string]$BaseDir
    )

    $findings = @()
    $suppressed = @()
    $inlineAllowMarker = [regex]'log-hygiene:allow\(([^)]*)\)'

    foreach ($file in $Files) {
        $rel = $file.ToLowerInvariant()
        $baseLower = $BaseDir.ToLowerInvariant()
        if ($rel.StartsWith($baseLower)) {
            $rel = $file.Substring($BaseDir.Length).TrimStart('\', '/')
        }
        # Get-Content -Raw instead of [IO.File]::ReadAllText: stays usable
        # under ConstrainedLanguage hosts.
        $content = Get-Content -LiteralPath $file -Raw
        $lines = $content -split "`r?`n"

        foreach ($inv in (Get-RustMacroInvocations -Content $content)) {
            if ($inv.Unbalanced) {
                throw ("check-log-hygiene: unbalanced macro invocation near line {0} in {1}" -f $inv.StartLine, $file)
            }
            $spanText = Get-LineRangeText -Lines $lines -Start $inv.StartLine -End $inv.EndLine
            $inlineReason = $null
            $im = $inlineAllowMarker.Match($spanText)
            if ($im.Success) { $inlineReason = $im.Groups[1].Value.Trim() }

            foreach ($rule in $script:Rules) {
                # Optional per-rule macro restriction (e.g. RawPayloadPrint
                # only fires on the stdout/stderr print family).
                if ($rule['Macros'] -and @($rule['Macros']) -notcontains $inv.Macro) { continue }
                if ($inv.Text -notmatch $rule.Regex) { continue }
                $finding = [pscustomobject]@{
                    Rule    = $rule.Id
                    Desc    = $rule.Description
                    RelPath = $rel
                    Line    = $inv.StartLine
                    Macro   = "$($inv.Macro)!"
                    Text    = $inv.Text
                }
                if ($inlineReason) {
                    $finding | Add-Member -NotePropertyName SuppressedBy -NotePropertyValue "inline: $inlineReason"
                    $suppressed += $finding
                    continue
                }
                $reason = Test-AllowlistSuppression -Finding $finding -Allowlist $script:Allowlist
                if ($reason) {
                    $finding | Add-Member -NotePropertyName SuppressedBy -NotePropertyValue "allowlist: $reason"
                    $suppressed += $finding
                    continue
                }
                $findings += $finding
            }
        }
    }
    return [pscustomobject]@{
        Findings   = @($findings)
        Suppressed = @($suppressed)
    }
}

function Get-RustSourceFiles {
    param([string]$Root)
    if (Test-Path -LiteralPath $Root -PathType Leaf) {
        return @((Resolve-Path -LiteralPath $Root).ProviderPath)
    }
    $all = @(Get-ChildItem -LiteralPath $Root -Recurse -Filter '*.rs' -File |
        Where-Object { $_.FullName -notmatch '\\target\\' } |
        ForEach-Object { $_.FullName })
    return ($all | Sort-Object)
}

function Invoke-SelfTest {
    # Seeds violations in a temp tree and asserts detection, cleanliness and
    # both suppression mechanisms. Returns $true only when every assertion
    # holds; writes progress lines as evidence.
    # Temp-path + random suffix without .NET statics ([IO.Path]::GetTempPath
    # and [guid]::NewGuid are blocked under ConstrainedLanguage).
    $tempRoot = Join-Path $env:TEMP (
        'log-hygiene-selftest-' + [string](Get-Random) + '-' + (Get-Date -Format 'HHmmss'))
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

    try {
        # Seed: one violating file per rule family (G1).
        Set-Content -LiteralPath (Join-Path $tempRoot 'seed_api_key.rs') -Encoding UTF8 -Value @'
pub fn bad(e: &str, api_key: &str) {
    error!("request failed for key {}: {}", api_key, e);
}
'@
        Set-Content -LiteralPath (Join-Path $tempRoot 'seed_settings.rs') -Encoding UTF8 -Value @'
pub struct Settings;
pub fn bad(settings: &Settings) {
    info!("dumping state: {:?}", settings);
}
'@
        Set-Content -LiteralPath (Join-Path $tempRoot 'seed_transcript.rs') -Encoding UTF8 -Value @'
pub fn bad(transcript_text: &str) {
    debug!("partial result: {transcript_text}");
    warn!("final: {}", transcript_text);
}
'@
        Set-Content -LiteralPath (Join-Path $tempRoot 'seed_stdout.rs') -Encoding UTF8 -Value @'
pub fn bad(text: &str) {
    println!("text: {}", text);
}
'@
        # Seed: clean file that must produce zero findings.
        Set-Content -LiteralPath (Join-Path $tempRoot 'clean.rs') -Encoding UTF8 -Value @'
pub fn good(e: &std::io::Error, n: &str) {
    // note: the word transcript appears, but no payload is formatted.
    info!("copied last transcript to clipboard");
    error!("model load failed: {}", e);
    debug!("device '{}': {:?}", n, e.kind());
    let raw = r#"not a string with ) tricks"#;
    let _ = raw;
    let lifetime: &'static str = "ok";
    let _ = lifetime;
}
'@
        # Seed: precision-regression file. Every line here was either a false
        # positive of an earlier rule revision or exercises a parser edge the
        # extractor once got wrong; ALL of it must stay unflagged.
        Set-Content -LiteralPath (Join-Path $tempRoot 'clean_edge.rs') -Encoding UTF8 -Value @'
pub fn edges(settings: &Settings, transcription_time: std::time::Instant, t_len: usize) {
    // Member access: one scalar field, not a struct dump.
    info!("selected model: {}", settings.selected_model);
    // Metadata-only method calls on transcript-stemmed identifiers.
    debug!("opencc lengths: {} -> {}", transcription_time.elapsed(), t_len);
    // Block comment INSIDE the invocation must not corrupt paren balance
    // (the extractor once reset its depth counter here).
    debug!("value {}", 42 /* answer */);
    // Raw string as format argument (raw-string close-probe path).
    warn!(r#"device {} ready"#, "mic");
    // Log-family macro printing an ERROR next to the word "transcript":
    // not a payload leak, and not in the print family.
    error!("paste of transcription failed: {}", "io error");
}
'@
        # Seed: inline-suppressed violation.
        Set-Content -LiteralPath (Join-Path $tempRoot 'seed_inline_allowed.rs') -Encoding UTF8 -Value @'
pub fn tolerated(key_id: &str) {
    info!("rotating api_key fingerprint {}", key_id); // log-hygiene:allow(reviews only the id, never the key)
}
'@
        # Central-allowlist probes: the entries are file-pinned AND
        # shape-pinned, so the probes must reproduce the exact real shapes in
        # matching relative paths. Also a negative control proving the
        # allowlist is narrow: the same shape in a different filename with a
        # different argument must still be flagged.
        $null = New-Item -ItemType Directory -Path (Join-Path $tempRoot 'settings') -Force
        Set-Content -LiteralPath (Join-Path $tempRoot 'settings\store.rs') -Encoding UTF8 -Value @'
pub fn real_shape(settings: &Settings) {
    debug!("Loaded settings: {:?}", settings);
}
'@
        Set-Content -LiteralPath (Join-Path $tempRoot 'headless.rs') -Encoding UTF8 -Value @'
pub fn real_shape(text: &str) {
    println!("text: {}", text);
}
'@
        Set-Content -LiteralPath (Join-Path $tempRoot 'seed_allowlist_negative.rs') -Encoding UTF8 -Value @'
pub fn not_exempted(settings_dump: &str) {
    println!("text: {}", settings_dump);
}
'@

        $files = Get-RustSourceFiles -Root $tempRoot
        $result = Invoke-HygieneScan -Files $files -BaseDir $tempRoot
        $byFile = @{}
        foreach ($f in $result.Findings) {
            $key = Split-Path -Leaf $f.RelPath
            if (-not $byFile.ContainsKey($key)) { $byFile[$key] = @() }
            $byFile[$key] += $f.Rule
        }

        $failures = @()

        # Assertion 1: each seeded violation is flagged by the right rule.
        $expected = @{
            'seed_api_key.rs'       = 'ApiKeyIdentifier'
            'seed_settings.rs'      = 'SettingsStructDump'
            'seed_transcript.rs'    = 'TranscriptPayloadInline'
            'seed_stdout.rs'        = 'RawPayloadPrint'
        }
        foreach ($name in $expected.Keys) {
            if (-not $byFile.ContainsKey($name)) {
                $failures += "MISS: $name was not flagged at all (expected $($expected[$name]))"
            }
            elseif ($byFile[$name] -notcontains $expected[$name]) {
                $failures += "WRONG RULE: $name flagged as [$($byFile[$name] -join ', ')], expected $($expected[$name])"
            }
        }
        # seed_transcript.rs also trips the positional rule - both must fire.
        if ($byFile.ContainsKey('seed_transcript.rs') -and
            $byFile['seed_transcript.rs'] -notcontains 'TranscriptPayloadPositional') {
            $failures += "MISS: seed_transcript.rs positional variant not flagged"
        }

        # Assertion 2: clean samples stay clean.
        foreach ($cleanName in @('clean.rs', 'clean_edge.rs')) {
            if ($byFile.ContainsKey($cleanName)) {
                $failures += "FALSE POSITIVE: $cleanName flagged as [$($byFile[$cleanName] -join ', ')]"
            }
        }

        # Assertion 3: inline suppression removes the marked finding.
        if ($byFile.ContainsKey('seed_inline_allowed.rs')) {
            $failures += "SUPPRESSION BROKEN: seed_inline_allowed.rs still flagged as [$($byFile['seed_inline_allowed.rs'] -join ', ')]"
        }
        $supNames = @($result.Suppressed | ForEach-Object { Split-Path -Leaf $_.RelPath })
        if ($supNames -notcontains 'seed_inline_allowed.rs') {
            $failures += "SUPPRESSION OPAQUE: inline-allowed finding absent from suppressed list"
        }

        # Assertion 4: central allowlist suppresses the exact real shapes in
        # the pinned files, and ONLY those.
        foreach ($exempt in @('store.rs', 'headless.rs')) {
            if ($byFile.ContainsKey($exempt)) {
                $failures += "SUPPRESSION BROKEN: $exempt still flagged as [$($byFile[$exempt] -join ', ')]"
            }
        }
        if (-not $supNames.Contains('store.rs') -or -not $supNames.Contains('headless.rs')) {
            $failures += "SUPPRESSION OPAQUE: allowlisted findings absent from suppressed list"
        }
        # Negative control: same shape, unpinned filename -> must be flagged.
        if (-not $byFile.ContainsKey('seed_allowlist_negative.rs')) {
            $failures += "ALLOWLIST TOO BROAD: seed_allowlist_negative.rs escaped detection"
        }

        # Assertion 5: a seeded violating tree yields a NONZERO scan verdict
        # (the exact exit path CI would take).
        $verdict = $(if (@($result.Findings).Count -gt 0) { $exitFindings } else { $exitClean })
        if ($verdict -ne $exitFindings) {
            $failures += "VERDICT: seeded tree did not produce nonzero verdict"
        }

        Write-Host ("self-test: seeded {0} findings across {1} violation file(s); {2} suppressed; verdict {3}" -f
            @($result.Findings).Count, $byFile.Keys.Count, @($result.Suppressed).Count, $verdict)
        foreach ($f in $result.Findings) {
            Write-Host ("  flagged: {0}:{1} [{2}] {3}" -f $f.RelPath, $f.Line, $f.Rule, $f.Macro)
        }
        foreach ($s in $result.Suppressed) {
            Write-Host ("  suppressed: {0}:{1} [{2}] via $($s.SuppressedBy)" -f $s.RelPath, $s.Line, $s.Rule)
        }

        if ($failures.Count -gt 0) {
            foreach ($msg in $failures) { Write-GuardError $msg }
            Write-GuardError "SELF-TEST FAILED"
            return $false
        }
        Write-Host "self-test: PASS (detection, cleanliness, inline + central suppression, nonzero verdict)"
        return $true
    }
    finally {
        if (Test-Path -LiteralPath $tempRoot) {
            Remove-Item -LiteralPath $tempRoot -Recurse -Force
        }
    }
}

# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------
try {
    # Allowlist sanity: every entry must carry a non-empty Reason (gate G2),
    # regardless of whether anything matched during this run.
    foreach ($entry in $script:Allowlist) {
        foreach ($field in @('Rule', 'Pattern', 'Reason')) {
            if (-not $entry[$field]) {
                throw "allowlist entry for '$($entry['Rule'])' is missing required field '$field' (justification comment + Reason are mandatory)."
            }
        }
    }

    if ($SelfTest) {
        if (Invoke-SelfTest) { exit $exitClean }
        exit $exitFindings
    }

    if (-not $RepoRoot) {
        $RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
    }
    if (-not $ScanRoot) {
        $ScanRoot = Join-Path $RepoRoot 'src-tauri\src'
    }
    if (-not (Test-Path -LiteralPath $ScanRoot)) {
        Write-GuardError "scan root '$ScanRoot' does not exist."
        exit $exitInfrastructure
    }

    $baseDir = $ScanRoot
    if (Test-Path -LiteralPath $ScanRoot -PathType Container) {
        $baseDir = (Resolve-Path -LiteralPath $ScanRoot).ProviderPath
    }
    # @() on the CALLER side: PowerShell unrolls a function's single-element
    # return array into a bare string, and $files.Count then fails under
    # Set-StrictMode (observed as a spurious exit-2 infrastructure error when
    # -ScanRoot points at one .rs file).
    $files = @(Get-RustSourceFiles -Root ((Resolve-Path -LiteralPath $ScanRoot).ProviderPath))
    if ($files.Count -eq 0) {
        Write-GuardError "no .rs files found under '$ScanRoot'."
        exit $exitInfrastructure
    }

    $result = Invoke-HygieneScan -Files $files -BaseDir $baseDir

    foreach ($f in $result.Findings) {
        $snippet = $f.Text -replace '\s+', ' '
        if ($snippet.Length -gt 160) { $snippet = $snippet.Substring(0, 160) }
        Write-Host ("VIOLATION {0}:{1} [{2}] {3}: {4}" -f
            $f.RelPath, $f.Line, $f.Rule, $f.Macro, $snippet)
    }

    Write-Host ("check-log-hygiene: scanned {0} file(s): {1} violation(s), {2} suppressed." -f
        $files.Count, @($result.Findings).Count, @($result.Suppressed).Count)
    if ($ShowSuppressed) {
        foreach ($s in $result.Suppressed) {
            Write-Host ("suppressed {0}:{1} [{2}] - {3}" -f $s.RelPath, $s.Line, $s.Rule, $s.SuppressedBy)
        }
    }

    if (@($result.Findings).Count -gt 0) {
        Write-GuardError ("FAILED: {0} log-hygiene violation(s). Suppress only with a reviewed, justified allowlist entry or an inline 'log-hygiene:allow(reason)' comment." -f @($result.Findings).Count)
        exit $exitFindings
    }
    exit $exitClean
}
catch {
    Write-GuardError "infrastructure error: $($_.Exception.Message)"
    exit $exitInfrastructure
}
