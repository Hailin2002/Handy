# Changes the Windows DEFAULT audio OUTPUT endpoint by automating the official
# Sound control panel (mmsys.cpl) - the same action a human performs in the UI.
# Used because the undocumented PolicyConfig COM interface refuses unelevated,
# non-packaged callers on Windows 11 24H2 (E_NOINTERFACE).
#
# Modes:
#   -Set -Name X       set the playback device whose name contains X as default
#   -RestoreTo X       alias of -Set (explicit revert marker)
#
# Usage:
#   pwsh -File set-default-output-uia.ps1 -Set -Name "Digital Output"
#   pwsh -File set-default-output-uia.ps1 -RestoreTo "Focusrite"
param(
    [switch]$Set,
    [string]$Name,
    [string]$RestoreTo
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$target = if ($Set) { $Name } elseif ($RestoreTo) { $RestoreTo } else { $null }
if (-not $target) { Write-Error 'specify -Set -Name <x> or -RestoreTo <x>' }

function Find-SoundWindow {
    for ($i = 0; $i -lt 40; $i++) {
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        $cond = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ClassNameProperty, '#32770')
        $windows = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
        foreach ($w in $windows) {
            $procId = $w.Current.ProcessId
            $proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
            if ($proc -and $proc.Name -match 'rundll32') { return $w }
        }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

Start-Process -FilePath 'rundll32.exe' -ArgumentList 'shell32.dll,Control_RunDLL mmsys.cpl,,0'
$win = Find-SoundWindow
if (-not $win) { throw 'Sound control panel window did not appear' }

try {
    # Locate the playback devices list (first List control in the dialog).
    $listCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::List)
    $list = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $listCond)
    if (-not $list) { throw 'device list not found in Sound dialog' }

    $itemCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::ListItem)
    $items = $list.FindAll([System.Windows.Automation.TreeScope]::Children, $itemCond)
    $selItem = $null
    foreach ($it in $items) {
        if ($it.Current.Name -like "*$target*") { $selItem = $it; break }
    }
    if (-not $selItem) {
        throw "no playback device matching '$target' (visible: $(($items | ForEach-Object { $_.Current.Name }) -join '; '))"
    }

    $selPat = $selItem.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
    $selPat.Select()
    Start-Sleep -Milliseconds 400

    # Click the Set Default button (name differs across locales).
    $btnCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)
    $buttons = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $btnCond)
    $invoked = $false
    foreach ($b in $buttons) {
        $n = $b.Current.Name
        if ($n -match '^(Set Default|Als Standard)$' -or $n -match 'Set Default|Als Standard') {
            ($b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)).Invoke()
            $invoked = $true
            break
        }
    }
    if (-not $invoked) { throw 'Set Default button not found' }
    Start-Sleep -Milliseconds 600
    Write-Output "default output device switched to match '*$target*' (selected: $($selItem.Current.Name))"
}
finally {
    # Close the dialog.
    try {
        $closeCond = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button)
        # Window close via WM_CLOSE is more reliable than hunting the X button:
        $proc = Get-Process -Id $win.Current.ProcessId -ErrorAction SilentlyContinue
        if ($proc) { $proc.CloseMainWindow() | Out-Null }
    } catch { }
}
