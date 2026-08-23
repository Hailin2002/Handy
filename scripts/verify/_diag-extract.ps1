# Diagnostic v2: logs progress to a file to locate the stall.
$ErrorActionPreference = 'Stop'
$log = Join-Path $env:TEMP 'diag-extract.log'
function L([string]$m) { Add-Content -LiteralPath $log -Value ("{0} {1}" -f (Get-Date -Format 'HH:mm:ss.fff'), $m) }
Set-Content -LiteralPath $log -Value 'start'

L 'reading source'
$src = Get-Content (Join-Path $PSScriptRoot '..\check-log-hygiene.ps1') -Raw
L ('source chars: ' + $src.Length)

L 'parsing'
$tokens = $null; $errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseInput($src, [ref]$tokens, [ref]$errors)
L ('parse errors: ' + @($errors).Count)

$f = $ast.FindAll(
    { param($a) $a -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $a.Name -eq 'Get-RustMacroInvocations' },
    $true)[0]
L ('function found: ' + ($null -ne $f))
$body = $f.Extent.Text

$marker = 'while ($i -lt $len -and -not $closed) {'
L ('marker found: ' + $body.Contains($marker))
$body = $body.Replace($marker, $marker + ' $script:Inner++')
$namesMarker = '$($script:MacroNames)'
L ('names marker found: ' + $body.Contains($namesMarker))
$body = $body.Replace($namesMarker, 'trace|debug|info|warn|error|println|print|eprintln|eprint|dbg|panic')

$fn = [scriptblock]::Create($body)
L 'scriptblock created'

$content = Get-Content (Join-Path $PSScriptRoot '..\..\src-tauri\src\managers\transcription.rs') -Raw
L ('content chars: ' + $content.Length)

$script:Inner = 0
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$inv = & $fn -Content $content
L ('elapsed ms: ' + $sw.ElapsedMilliseconds)
L ('inner iterations: ' + $script:Inner)
L ('invocations: ' + @($inv).Count)
L ('unbalanced: ' + @($inv | Where-Object { $_.Unbalanced }).Count)
L 'done'
