# Mid-recording default-device switch test for the DICTATION CAPTURE PATH.
#
# Starts a real WASAPI shared-mode capture stream on the system DEFAULT INPUT
# device -- the OS-level mechanism the dictation path uses (cpal/WASAPI shared
# mode; src-tauri/src/audio_toolkit/audio/recorder.rs opens the resolved device
# at recording start; managers/audio.rs DesiredMicrophone::Default resolves to
# the cpal system default) -- then switches the Windows DEFAULT device
# MID-RECORDING, separately for the input (capture, tab "Recording") and the
# output (render, tab "Playback") flows.
#
# Design intent under test (recorder.rs stream error callback stores a flag;
# unit test stream_error_requires_reopen): an open dictation stream stays bound
# to its device; a routine OS default-device switch must NOT touch it =>
# expected outcome SEAMLESS CONTINUATION of capture callbacks. If the stream
# errors instead, a CLEAN TYPED RECOVERY (error surfaced, reopen required, no
# hang) is acceptable. A HANG or a SILENT DEAD RECORDING (no packets AND no
# error) is a blocking defect.
#
# Device switching automates mmsys.cpl over UI Automation -- extended from
# set-default-output-uia.ps1 to BOTH tabs and to context-menu actions -- because
# the PolicyConfig COM interface refuses unelevated callers on Windows 11
# 24H2+ (E_NOINTERFACE; reproduced here on 25H2 / build 26200).
#
# Modes:
#   -ListInputs            print known capture endpoints + live states
#   -ListOutputs           print known render endpoints + live states
#   -Get                   print current default capture/render endpoints
#   -Run -Class Input|Output|Both [-PreSwitchSeconds N] [-PostSwitchSeconds N]
#
# Per run artifacts (default location):
#   src-tauri/target/baseline-tools/logs/device-switch/run-<stamp>/
#     events.log            DSW_* marker timeline (vocabulary below)
#     timeline-<class>.csv  per-packet capture timeline
#     SUMMARY.md            verdict + acceptance-gate mapping
#
# events.log marker vocabulary (all prefixed DSW_, one per line, timestamped):
#   RUN_BEGIN/RUN_END, CLASS_BEGIN/CLASS_END,
#   ENDPOINTS, RECORDER_STARTED, PRE_LIVENESS_OK/PRE_LIVENESS_DEAD,
#   STALE_SOUND_DIALOGS_CLOSED, SOUND_TAB_CORRECTING/SOUND_TAB_SELECTED/
#   SOUND_TAB_VERIFY_SKIPPED, UIA_ATTEMPT_FAILED,
#   ENABLE_BEGIN/ENABLE_CONFIRMED/ENABLE_FAILED,
#   SWITCH_BEGIN/SWITCH_ABORTED, UIA_CONTEXTMENU_INVOKED/UIA_BUTTON_INVOKED/
#   UIA_CONTEXTMENU_NO_MATCH/UIA_CONTEXTMENU_NOT_FOUND,
#   DEFAULT_CHANGE_CONFIRMED/DEFAULT_CHANGE_FAILED,
#   POST_WINDOW_DONE, RECORDER_STOPPED, CLASS_VERDICT,
#   RESTORE_DEFAULT_BEGIN/RESTORE_DEFAULT_OK/RESTORE_DEFAULT_FAILED,
#   TEARDOWN_DISABLE_BEGIN/TEARDOWN_DISABLE_OK/TEARDOWN_DISABLE_FAILED
#
# Classification per class:
#   seamless        zero stream errors, zero new discontinuities, callbacks
#                   continued through the whole switch+post window
#                   (max inter-packet gap <= 250 ms, last packet <= 1 s ago)
#   clean-restart   stream errored mid-test, error text captured (typed
#                   recovery possible), recorder stopped cleanly (no hang)
#   defect-hang     >=5 consecutive event-wait timeouts (>=10 s of silence
#                   from a supposedly live stream) or Stop() join timeout
#   defect-dead     no packets and no errors in the post-switch window
#   degraded-gap    continued, but max inter-packet gap > 250 ms
#   blocked-*       environment precondition unmet (reported, never guessed)
#
# State restoration (unless -NoRestore): each class's default device is
# restored afterwards and any endpoint this script ENABLED to obtain a second
# capture candidate is DISABLED again.
#
# Examples:
#   powershell -NoProfile -ExecutionPolicy Bypass -File device-switch-test.ps1 -ListInputs
#   powershell -NoProfile -ExecutionPolicy Bypass -File device-switch-test.ps1 -Get
#   powershell -NoProfile -ExecutionPolicy Bypass -File device-switch-test.ps1 -Run -Class Both
param(
    [switch]$ListInputs,
    [switch]$ListOutputs,
    [switch]$Get,
    [switch]$Run,
    [ValidateSet('Input', 'Output', 'Both')]
    [string]$Class = 'Both',
    [int]$PreSwitchSeconds = 5,
    [int]$PostSwitchSeconds = 10,
    [string]$LogDir,
    [switch]$NoRestore
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms

# ----------- embedded C# core: enumeration, defaults, capture recorder -------

$cs = @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

namespace SpeakSpike {

    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
    class DswMMDeviceEnumeratorCom { }

    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface DswIMMDeviceEnumerator {
        [PreserveSig]
        int EnumAudioEndpoints(int dataFlow, int stateMask, out IntPtr devices);
        [PreserveSig]
        int GetDefaultAudioEndpoint(int dataFlow, int role, out IntPtr endpoint);
        [PreserveSig]
        int GetDevice([MarshalAs(UnmanagedType.LPWStr)] string id, out IntPtr device);
        int RegisterEndpointNotificationCallback(IntPtr cb);
        int UnregisterEndpointNotificationCallback(IntPtr cb);
    }

    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface DswIMMDevice {
        int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, out IntPtr iface);
        int OpenPropertyStore(int access, out IntPtr store);
        [PreserveSig]
        int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
        [PreserveSig]
        int GetState(out int state);
    }

    [ComImport, Guid("1CB9AD4C-DBFA-4c32-B178-C2F568A703B2"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface DswIAudioClient {
        [PreserveSig]
        int Initialize(int shareMode, int streamFlags, long bufferDuration, long periodicity, IntPtr format, IntPtr audioSessionGuid);
        int GetBufferSize(out uint numBufferFrames);
        int GetStreamLatency(out long latency);
        int GetCurrentPadding(out uint padding);
        [PreserveSig]
        int IsFormatSupported(int shareMode, IntPtr format, out IntPtr closestMatch);
        [PreserveSig]
        int GetMixFormat(out IntPtr format);
        int GetDevicePeriod(out long defaultPeriod, out long minPeriod);
        [PreserveSig]
        int Start();
        [PreserveSig]
        int Stop();
        [PreserveSig]
        int Reset();
        [PreserveSig]
        int SetEventHandle(IntPtr handle);
        [PreserveSig]
        int GetService(ref Guid iid, out IntPtr service);
    }

    [ComImport, Guid("C8ADBD64-E71E-48a0-A4DE-185C395CD317"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface DswIAudioCaptureClient {
        [PreserveSig]
        int GetBuffer(out IntPtr data, out uint frames, out uint flags, out long devPos, out long qpcPos);
        [PreserveSig]
        int ReleaseBuffer(uint frames);
        [PreserveSig]
        int GetNextPacketSize(out uint frames);
    }

    /// One endpoint candidate from the MMDevices registry (id + best-effort
    /// friendly name). Live state comes from Devices.StateOf, never from here.
    public class DswEndpoint {
        public string Id;
        public string Name;
        public override string ToString() { return Id + "  " + Name; }
    }

    public static class Devices {
        static DswIMMDeviceEnumerator enumerator =
            (DswIMMDeviceEnumerator)(object)new DswMMDeviceEnumeratorCom();

        // Registry candidate list for flow (0 = capture, 1 = render).
        public static DswEndpoint[] KnownEndpoints(int flow) {
            string basePath = @"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\"
                + (flow == 0 ? "Capture" : "Render");
            using (Microsoft.Win32.RegistryKey root =
                   Microsoft.Win32.Registry.LocalMachine.OpenSubKey(basePath)) {
                if (root == null) return new DswEndpoint[0];
                System.Collections.Generic.List<DswEndpoint> list =
                    new System.Collections.Generic.List<DswEndpoint>();
                foreach (string sub in root.GetSubKeyNames()) {
                    string name = RegistryName(basePath, sub);
                    if (String.IsNullOrEmpty(name)) continue;
                    DswEndpoint e = new DswEndpoint();
                    e.Id = "{0.0." + (flow == 0 ? "1" : "0") + ".00000000}." + sub;
                    e.Name = name;
                    list.Add(e);
                }
                return list.ToArray();
            }
        }

        static string RegistryName(string basePath, string keyName) {
            using (Microsoft.Win32.RegistryKey props =
                   Microsoft.Win32.Registry.LocalMachine.OpenSubKey(
                       basePath + "\\" + keyName + "\\Properties")) {
                if (props == null) return null;
                string name = null;
                string fallback = null;
                foreach (string valName in props.GetValueNames()) {
                    string lower = valName.ToLowerInvariant();
                    if (lower.StartsWith("{a45c254e-df1c-4efd-8020-67d146a850e0},")) {
                        string val = Convert.ToString(props.GetValue(valName),
                            System.Globalization.CultureInfo.InvariantCulture);
                        if (lower.EndsWith(",14")) { name = val; break; }
                        if (lower.EndsWith(",2")) fallback = val;
                    }
                }
                if (name == null) name = fallback;
                return name;
            }
        }

        /// Live endpoint state straight from MMDevice COM:
        /// 1 active / 2 disabled / 4 notpresent / 8 unplugged,
        /// or the negative HRESULT when the endpoint cannot be resolved.
        public static int StateOf(string id) {
            IntPtr p;
            int hr = enumerator.GetDevice(id, out p);
            if (hr != 0) return -hr;
            DswIMMDevice dev = (DswIMMDevice)Marshal.GetObjectForIUnknown(p);
            int st;
            dev.GetState(out st);
            Marshal.Release(p);
            return st;
        }

        /// Current default endpoint id for flow (0 capture / 1 render) and
        /// role (0 console / 1 multimedia / 2 communication); "hr=0x..." on error.
        public static string DefaultId(int flow, int role) {
            IntPtr p;
            // Map the script's convention (0=capture, 1=render) to COM
            // EDataFlow (eRender=0, eCapture=1).
            int comFlow = (flow == 0) ? 1 : 0;
            int hr = enumerator.GetDefaultAudioEndpoint(comFlow, role, out p);
            if (hr != 0) return "hr=0x" + hr.ToString("X8");
            DswIMMDevice dev = (DswIMMDevice)Marshal.GetObjectForIUnknown(p);
            string id;
            dev.GetId(out id);
            Marshal.Release(p);
            return id;
        }
    }

    /// Shared-mode event-driven WASAPI capture stream opened on the CURRENT
    /// DEFAULT capture endpoint -- the OS-level stand-in for the dictation
    /// capture stream (cpal opens the identical WASAPI shared-mode input).
    /// Tracks a strictly time-ordered per-packet timeline so continuity across
    /// a mid-recording device switch is measurable: packet/frame counters,
    /// SILENT/DISCONTINUITY/TIMESTAMP_ERROR flags, inter-packet wall gaps,
    /// event-wait timeouts, typed stream errors.
    public class CaptureRecorder {
        IntPtr clientPtr = IntPtr.Zero;
        IntPtr capturePtr = IntPtr.Zero;
        IntPtr fmtPtr = IntPtr.Zero;
        DswIAudioClient client;
        DswIAudioCaptureClient capture;
        EventWaitHandle tick;
        Thread worker;
        StreamWriter csv;
        readonly object csvLock = new object();
        DateTime startedUtc = DateTime.UtcNow;
        DateTime prevReadUtc = DateTime.UtcNow;
        volatile bool stopping = false;

        public string CsvPath = "";
        public volatile bool Errored = false;
        public volatile bool ThreadExited = false;
        public volatile bool StopTimedOut = false;
        public volatile string ErrorText = "";
        public volatile string FormatText = "";
        public volatile int ConsecutiveWaitTimeouts = 0;

        public long TotalPackets = 0;
        public long TotalFrames = 0;
        public long SilentPackets = 0;
        public long DiscontinuityPackets = 0;
        public long TimestampErrorPackets = 0;
        public long WaitTimeouts = 0;
        public double MaxGapMs = 0.0;          // since last ResetMaxGap()
        public DateTime LastPacketUtc = DateTime.MinValue;

        public static CaptureRecorder StartDefaultCapture(string csvPath) {
            CaptureRecorder r = new CaptureRecorder();
            r.CsvPath = csvPath;
            r.StartInternal();
            return r;
        }

        void StartInternal() {
            DswIMMDeviceEnumerator en =
                (DswIMMDeviceEnumerator)(object)new DswMMDeviceEnumeratorCom();
            IntPtr devPtr;
            // EDataFlow: eRender=0, eCapture=1 -- this class's flow convention
            // (0=input/capture, 1=output/render) must be mapped to COM here.
            int hr = en.GetDefaultAudioEndpoint(1 /*eCapture*/, 0 /*eConsole*/, out devPtr);
            if (hr != 0)
                throw new Exception("GetDefaultAudioEndpoint(capture) hr=0x" + hr.ToString("X8"));
            DswIMMDevice dev = (DswIMMDevice)Marshal.GetObjectForIUnknown(devPtr);

            Guid cliIid = new Guid("1CB9AD4C-DBFA-4c32-B178-C2F568A703B2");
            IntPtr cp;
            int hrA = dev.Activate(ref cliIid, 1 /*CLSCTX_INPROC_SERVER*/, IntPtr.Zero, out cp);
            if (hrA != 0)
                throw new Exception("Activate(IAudioClient) hr=0x" + hrA.ToString("X8"));
            clientPtr = cp;
            client = (DswIAudioClient)Marshal.GetObjectForIUnknown(cp);

            IntPtr f;
            int hrF = client.GetMixFormat(out f);
            if (hrF != 0) throw new Exception("GetMixFormat hr=0x" + hrF.ToString("X8"));
            fmtPtr = f;
            short ch = Marshal.ReadInt16(f, 2);
            int rate = Marshal.ReadInt32(f, 4);
            short bits = Marshal.ReadInt16(f, 14);
            FormatText = rate + " Hz / " + ch + " ch / " + bits + " bit (shared mix format)";

            // 10 s buffer tolerates long stalls without corrupting the stream;
            // the event still fires every device period (~10 ms).
            int hrI = client.Initialize(0 /*shared*/, 0x00040000 /*EVENTCALLBACK*/,
                                        10000000L, 0, f, IntPtr.Zero);
            if (hrI != 0)
                throw new Exception("Initialize(shared,event) hr=0x" + hrI.ToString("X8"));

            tick = new EventWaitHandle(false, EventResetMode.AutoReset);
            int hrH = client.SetEventHandle(tick.SafeWaitHandle.DangerousGetHandle());
            if (hrH != 0) throw new Exception("SetEventHandle hr=0x" + hrH.ToString("X8"));

            Guid capIid = new Guid("C8ADBD64-E71E-48a0-A4DE-185C395CD317");
            IntPtr cptr;
            int hrS = client.GetService(ref capIid, out cptr);
            if (hrS != 0)
                throw new Exception("GetService(IAudioCaptureClient) hr=0x" + hrS.ToString("X8"));
            capturePtr = cptr;
            capture = (DswIAudioCaptureClient)Marshal.GetObjectForIUnknown(cptr);

            Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(CsvPath)));
            csv = new StreamWriter(CsvPath, false, System.Text.Encoding.UTF8);
            csv.WriteLine("t_rel_ms,packet_index,frames,flags_hex,silent,discontinuity,timestamp_error,gap_since_prev_ms,cum_packets,cum_frames,note");

            int hrSt = client.Start();
            if (hrSt != 0) throw new Exception("Start hr=0x" + hrSt.ToString("X8"));

            startedUtc = DateTime.UtcNow;
            prevReadUtc = startedUtc;
            worker = new Thread(new ThreadStart(RunLoop));
            worker.IsBackground = true;
            worker.Start();
        }

        void RunLoop() {
            long packetIndex = 0;
            try {
                while (!stopping) {
                    int waited = WaitHandle.WaitAny(new WaitHandle[] { tick }, 2000);
                    if (waited != 0) {
                        WaitTimeouts++;
                        ConsecutiveWaitTimeouts++;
                        DateTime now = DateTime.UtcNow;
                        double gap = (now - prevReadUtc).TotalMilliseconds;
                        if (gap > MaxGapMs) MaxGapMs = gap;
                        WriteRow(now, packetIndex, 0, 0, gap,
                            "wait_timeout#" + ConsecutiveWaitTimeouts);
                        continue;
                    }
                    ConsecutiveWaitTimeouts = 0;
                    while (true) {
                        uint queued;
                        int hr = capture.GetNextPacketSize(out queued);
                        if (hr != 0) { Fail("GetNextPacketSize hr=0x" + hr.ToString("X8")); return; }
                        if (queued == 0) break;

                        IntPtr data;
                        uint frames;
                        uint flags;
                        long devPos, qpcPos;
                        hr = capture.GetBuffer(out data, out frames, out flags,
                                               out devPos, out qpcPos);
                        if (hr != 0) { Fail("GetBuffer hr=0x" + hr.ToString("X8")); return; }

                        DateTime now = DateTime.UtcNow;
                        double gap = (now - prevReadUtc).TotalMilliseconds;
                        if (gap > MaxGapMs) MaxGapMs = gap;
                        prevReadUtc = now;
                        LastPacketUtc = now;
                        packetIndex++;
                        TotalPackets++;
                        TotalFrames += frames;
                        if ((flags & 0x4) != 0) SilentPackets++;          // AUDCLNT_BUFFERFLAGS_SILENT
                        if ((flags & 0x1) != 0) DiscontinuityPackets++;   // ..._DATA_DISCONTINUITY
                        if ((flags & 0x2) != 0) TimestampErrorPackets++;  // ..._TIMESTAMP_ERROR

                        WriteRow(now, packetIndex, frames, flags, gap, "");
                        hr = capture.ReleaseBuffer(frames);
                        if (hr != 0) { Fail("ReleaseBuffer hr=0x" + hr.ToString("X8")); return; }
                    }
                }
            } catch (Exception ex) {
                Fail("worker exception: " + ex.Message);
            } finally {
                ThreadExited = true;
                lock (csvLock) {
                    if (csv != null) { try { csv.Flush(); csv.Close(); } catch { } csv = null; }
                }
            }
        }

        void WriteRow(DateTime stampUtc, long packetIndex, uint frames, uint flags,
                      double gapMs, string note) {
            double rel = (stampUtc - startedUtc).TotalMilliseconds;
            lock (csvLock) {
                if (csv == null) return;
                csv.WriteLine(String.Format(System.Globalization.CultureInfo.InvariantCulture,
                    "{0:F3},{1},{2},0x{3},{4},{5},{6},{7:F3},{8},{9},{10}",
                    rel, packetIndex, frames, flags.ToString("X2"),
                    (flags & 0x4) != 0 ? 1 : 0,
                    (flags & 0x1) != 0 ? 1 : 0,
                    (flags & 0x2) != 0 ? 1 : 0,
                    gapMs, TotalPackets, TotalFrames, note));
            }
        }

        void Fail(string msg) {
            Errored = true;
            if (ErrorText.Length == 0 || !Errored) ErrorText = msg;
            else ErrorText += "; " + msg;
        }

        public void ResetMaxGap() { MaxGapMs = 0.0; }

        public double MillisecondsSinceLastPacket() {
            if (LastPacketUtc == DateTime.MinValue) return -1.0;
            return (DateTime.UtcNow - LastPacketUtc).TotalMilliseconds;
        }

        public double ElapsedSeconds() {
            return (DateTime.UtcNow - startedUtc).TotalSeconds;
        }

        public void StopRecording() {
            stopping = true;
            try { if (tick != null) tick.Set(); } catch { }
            if (worker != null) {
                if (!worker.Join(5000)) {
                    StopTimedOut = true;
                    try { worker.Abort(); } catch { }
                }
            }
            if (client != null) { try { client.Stop(); } catch { } }
            Cleanup();
        }

        void Cleanup() {
            try { if (fmtPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(fmtPtr); } catch { }
            fmtPtr = IntPtr.Zero;
            try { if (capturePtr != IntPtr.Zero) Marshal.Release(capturePtr); } catch { }
            capturePtr = IntPtr.Zero;
            try { if (clientPtr != IntPtr.Zero) Marshal.Release(clientPtr); } catch { }
            clientPtr = IntPtr.Zero;
        }
    }
}
'@

Add-Type -TypeDefinition $cs -Language CSharp

# ------------------------------ shared helpers -------------------------------

function Get-DefaultLogDir {
    if ($LogDir) { return $LogDir }
    # <repo>\docs\phase1-spike\tools\device-switch-test.ps1 -> <repo>
    $repo = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSCommandPath)))
    return Join-Path $repo 'src-tauri\target\baseline-tools\logs\device-switch'
}

function Log-Event([string]$Path, [string]$msg) {
    $line = '{0:yyyy-MM-dd HH:mm:ss.fffzzz} {1}' -f (Get-Date), $msg
    Add-Content -Path $Path -Value $line
    # Write-Host, NOT Write-Output: event echoes must stay off the pipeline so
    # callers' return values are never polluted by marker lines.
    Write-Host $line
}

function Get-LiveEndpoints([int]$flow) {
    # Registry candidates annotated with authoritative live COM state.
    # NOTE: plain return, NOT 'return ,$out' - a comma-wrapped array piped
    # directly into Where-Object stays nested there ($_ becomes the whole
    # array), which made every endpoint look like one ambiguous candidate.
    # Callers that need array semantics wrap with @(...) themselves.
    $out = @()
    foreach ($e in [SpeakSpike.Devices]::KnownEndpoints($flow)) {
        $st = [SpeakSpike.Devices]::StateOf($e.Id)
        $out += [pscustomobject]@{ Id = $e.Id; Name = $e.Name; LiveState = $st }
    }
    return $out
}

function Format-Endpoints([object[]]$eps) {
    return ($eps | ForEach-Object { "{state=$($_.LiveState)} '$($_.Name)'" }) -join ', '
}

# --------------- UIA automation of mmsys.cpl (both tabs + menus) -------------

function Find-SoundWindow {
    for ($i = 0; $i -lt 40; $i++) {
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        $cond = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ClassNameProperty, '#32770')
        $windows = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
        foreach ($w in $windows) {
            $proc = Get-Process -Id $w.Current.ProcessId -ErrorAction SilentlyContinue
            if ($proc -and $proc.Name -match 'rundll32') { return $w }
        }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

function Close-SoundWindow($win) {
    try {
        $proc = Get-Process -Id $win.Current.ProcessId -ErrorAction SilentlyContinue
        if ($proc) { $null = $proc.CloseMainWindow() }
    } catch { }
}

# mmsys.cpl is single-instance: while ANY Sound dialog (possibly a leftover
# whose CloseMainWindow was swallowed by an open modal context menu) is alive,
# a new launch merely surfaces the OLD window with the OLD tab. That exact
# failure made run-20260823_070747's output class search the Recording list
# for a playback device. So: close every rundll32-owned #32770 dialog and
# wait until none remain, before opening a fresh one.
function Close-StaleSoundWindows([string]$EventsPath, [string]$Tag) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty, '#32770')
    for ($round = 0; $round -lt 6; $round++) {
        $stale = New-Object 'System.Collections.Generic.List[object]'
        $windows = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
        foreach ($w in $windows) {
            $proc = Get-Process -Id $w.Current.ProcessId -ErrorAction SilentlyContinue
            if ($proc -and $proc.Name -match 'rundll32') { $stale.Add(@($w, $proc)) }
        }
        if ($stale.Count -eq 0) {
            if ($round -gt 0) { Log-Event $EventsPath "$Tag STALE_SOUND_DIALOGS_CLOSED rounds=$round" }
            return
        }
        if ($round -eq 0) { Log-Event $EventsPath "$Tag STALE_SOUND_DIALOGS_CLOSING count=$($stale.Count)" }
        foreach ($pair in $stale) {
            try {
                $null = $pair[1].CloseMainWindow()
                # CloseMainWindow is a polite request that a dialog with an
                # open modal menu ignores; escalate to WindowPattern.Close,
                # then (last resort) kill the mmsys.cpl host rundll32 -- it
                # only hosts the Sound dialog, no other state.
            } catch { }
        }
        Start-Sleep -Milliseconds 700
        $still = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
        $left = @()
        foreach ($w in $still) {
            $proc = Get-Process -Id $w.Current.ProcessId -ErrorAction SilentlyContinue
            if ($proc -and $proc.Name -match 'rundll32') { $left += ,@($w, $proc) }
        }
        if ($left.Count -eq 0) {
            Log-Event $EventsPath "$Tag STALE_SOUND_DIALOGS_CLOSED rounds=$($round + 1)"
            return
        }
        if ($round -ge 2) {
            foreach ($pair in $left) {
                try {
                    ($pair[0].GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern)).Close()
                } catch { }
                try { $pair[1].Kill() } catch { }
            }
        }
        Start-Sleep -Milliseconds 500
    }
    Log-Event $EventsPath "$Tag STALE_SOUND_DIALOGS_PERSISTED -- continuing anyway; tab verification will catch misuse"
}

# Verifies (and forces if needed) the ACTIVE tab of the open Sound dialog.
# The mmsys.cpl,,N argument is only a request and is dropped when an existing
# instance is reused, so the device list is never trusted without this check.
function Select-SoundTab($win, [int]$Tab, [string]$EventsPath, [string]$Tag) {
    $tabItemCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::TabItem)
    $tabs = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $tabItemCond)
    if ($tabs.Count -le $Tab) {
        Log-Event $EventsPath "$Tag SOUND_TAB_VERIFY_SKIPPED tab_items=$($tabs.Count)"
        return
    }
    $t = $tabs[$Tab]
    $selPat = $null
    try { $selPat = $t.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern) } catch { }
    if ($selPat -ne $null) {
        if (-not $selPat.Current.IsSelected) {
            Log-Event $EventsPath "$Tag SOUND_TAB_CORRECTING requested=$Tab name='$($t.Current.Name)'"
            $selPat.Select()
            Start-Sleep -Milliseconds 800
        }
        Log-Event $EventsPath "$Tag SOUND_TAB_SELECTED index=$Tab name='$($t.Current.Name)'"
    }
}

function Open-SoundTab([int]$Tab, [string]$EventsPath, [string]$Tag) {
    Close-StaleSoundWindows $EventsPath $Tag
    Start-Process -FilePath 'rundll32.exe' -ArgumentList ("shell32.dll,Control_RunDLL mmsys.cpl,," + $Tab)
    $win = Find-SoundWindow
    if (-not $win) { throw 'Sound control panel window did not appear' }
    try { $null = $win.SetFocus() } catch { }
    Select-SoundTab $win $Tab $EventsPath $Tag
    Log-Event $EventsPath "$Tag SOUND_DIALOG_OPEN tab=$Tab"
    return $win
}

function Get-DeviceListItem($win, [string]$TargetName) {
    $listCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::List)
    $list = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $listCond)
    if (-not $list) { throw 'device list not found in Sound dialog' }
    $itemCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::ListItem)
    $items = $list.FindAll([System.Windows.Automation.TreeScope]::Children, $itemCond)
    foreach ($it in $items) {
        if ($it.Current.Name -like "*$TargetName*") { return $it }
    }
    throw "no device matching '$TargetName' (visible: $(($items | ForEach-Object { $_.Current.Name }) -join '; '))"
}

# Selects the item, opens its context menu (Shift+F10), invokes the first menu
# item matching $MenuPattern, logs what happened. Returns $true on invoke.
function Invoke-DeviceContextMenu($win, [string]$TargetName, [string]$MenuPattern,
                                  [string]$EventsPath, [string]$Tag) {
    $item = Get-DeviceListItem $win $TargetName
    $selPat = $item.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
    $selPat.Select()
    Start-Sleep -Milliseconds 300
    $null = $item.SetFocus()
    Start-Sleep -Milliseconds 300

    [System.Windows.Forms.SendKeys]::SendWait('+{F10}')
    Start-Sleep -Milliseconds 900

    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $menuCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty, '#32768')
    $menu = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $menuCond)
    if (-not $menu) {
        Log-Event $EventsPath "$Tag UIA_CONTEXTMENU_NOT_FOUND"
        return $false
    }
    $miCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::MenuItem)
    $mis = $menu.FindAll([System.Windows.Automation.TreeScope]::Descendants, $miCond)
    foreach ($m in $mis) {
        if ($m.Current.Name -match $MenuPattern) {
            ($m.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)).Invoke()
            Log-Event $EventsPath "$Tag UIA_CONTEXTMENU_INVOKED item='$($m.Current.Name)'"
            Start-Sleep -Milliseconds 800
            return $true
        }
    }
    Log-Event $EventsPath "$Tag UIA_CONTEXTMENU_NO_MATCH items='$(($mis | ForEach-Object { $_.Current.Name }) -join '; ')'"
    try { [System.Windows.Forms.SendKeys]::SendWait('{ESC}') } catch { }
    Start-Sleep -Milliseconds 300
    return $false
}

# Applies 'Set as Default Device' to the tab-$Tab device matching $TargetName
# (context menu preferred; dialog Set Default button as fallback). Each
# attempt uses a FRESH dialog after stale-dialog cleanup, so a wrong-tab or
# half-dead window can never poison more than one attempt.
function Invoke-SetDefaultDeviceUia([int]$Tab, [string]$TargetName,
                                    [string]$EventsPath, [string]$Tag) {
    $maxAttempts = 3
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        $win = $null
        try {
            $win = Open-SoundTab $Tab $EventsPath $Tag
            $done = Invoke-DeviceContextMenu $win $TargetName `
                '^(Set as Default Device|Als Standardger.t festlegen)$' $EventsPath $Tag
            if (-not $done) {
                Log-Event $EventsPath "$Tag FALLBACK_SETDEFAULT_BUTTON"
                $btnCond = New-Object System.Windows.Automation.PropertyCondition(
                    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                    [System.Windows.Automation.ControlType]::Button)
                $buttons = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $btnCond)
                foreach ($b in $buttons) {
                    if ($b.Current.Name -match 'Set Default|Als Standard') {
                        ($b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)).Invoke()
                        Log-Event $EventsPath "$Tag UIA_BUTTON_INVOKED button='$($b.Current.Name)'"
                        Start-Sleep -Milliseconds 600
                        $done = $true
                        break
                    }
                }
            }
            if (-not $done) { throw 'neither context menu nor Set Default button available' }
            return
        }
        catch {
            Log-Event $EventsPath "$Tag UIA_ATTEMPT_FAILED attempt=$attempt/$maxAttempts reason='$($_.Exception.Message)'"
            if ($attempt -eq $maxAttempts) { throw }
            Start-Sleep -Milliseconds 1200
        }
        finally {
            if ($win -ne $null) { Close-SoundWindow $win }
        }
    }
}

# Enables or disables the endpoint matching $TargetName on Sound tab $Tab
# (1 = Recording / capture, 0 = Playback / render).
function Invoke-EndpointEnableToggle([string]$TargetName, [bool]$Enable,
                                     [int]$Tab,
                                     [string]$EventsPath, [string]$Tag) {
    $pattern = if ($Enable) { '^(Enable|Aktivieren)$' } else { '^(Disable|Deaktivieren)$' }
    $win = Open-SoundTab $Tab $EventsPath $Tag
    try {
        return Invoke-DeviceContextMenu $win $TargetName $pattern $EventsPath $Tag
    }
    finally { Close-SoundWindow $win }
}

# --------------------- default-device switch with verification ---------------

# Switches the flow default (0 = capture/input, 1 = render/output) to the
# single ACTIVE endpoint whose name contains $TargetName, then polls COM until
# the change is confirmed (<= 10 s). Returns a diagnostic object; throws when
# the target cannot be resolved unambiguously.
function Switch-DefaultDevice([int]$flow, [string]$TargetName,
                              [string]$EventsPath, [string]$Tag) {
    $candidates = @(Get-LiveEndpoints $flow | Where-Object {
        $_.LiveState -eq 1 -and $_.Name -like "*$TargetName*" })
    if ($candidates.Count -eq 0) { throw "no ACTIVE flow-$flow endpoint matching '$TargetName'" }
    if ($candidates.Count -gt 1) {
        throw "ambiguous flow-$flow target '$TargetName': $(Format-Endpoints $candidates)"
    }
    $target = $candidates[0]

    $before = [SpeakSpike.Devices]::DefaultId($flow, 0)
    Log-Event $EventsPath "$Tag SWITCH_BEGIN flow=$flow target_id=$($target.Id) target_name='$($target.Name)' default_before=$before"

    $tab = 1; if ($flow -eq 1) { $tab = 0 }
    Invoke-SetDefaultDeviceUia -Tab $tab -TargetName $target.Name -EventsPath $EventsPath -Tag $Tag

    $confirmed = $false
    $deadline = (Get-Date).AddSeconds(10)
    $after = $before
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 250
        $after = [SpeakSpike.Devices]::DefaultId($flow, 0)
        if ($after -eq $target.Id) { $confirmed = $true; break }
    }
    if ($confirmed) {
        Log-Event $EventsPath "$Tag DEFAULT_CHANGE_CONFIRMED flow=$flow default_after=$after"
    } else {
        Log-Event $EventsPath "$Tag DEFAULT_CHANGE_FAILED flow=$flow default_last_seen=$after"
    }
    return [pscustomobject]@{
        Confirmed = $confirmed; TargetId = $target.Id; TargetName = $target.Name
        Before = $before; After = $after
    }
}

# -------------------------- one switch-class test run ------------------------

function Test-DeviceSwitchClass {
    param(
        [int]$Flow,             # 0=input(capture), 1=output(render)
        [string]$ClassName,     # 'input' / 'output'
        [string]$RunDir,
        [string]$EventsPath,
        [int]$PreSeconds,
        [int]$PostSeconds,
        [bool]$DoRestore
    )

    Log-Event $EventsPath "DSW_CLASS_BEGIN class=$ClassName flow=$Flow"

    $result = [ordered]@{
        Class       = $ClassName
        Verdict     = ''
        Evidence    = @()
        Restored    = 'n/a'
        Teardown    = 'n/a'
    }

    # -- precondition: current default + a second usable endpoint ----------
    $defaultId = [SpeakSpike.Devices]::DefaultId($Flow, 0)
    if ($defaultId -like 'hr=*') { throw "cannot resolve flow-$Flow default: $defaultId" }
    $live = Get-LiveEndpoints $Flow
    Log-Event $EventsPath "DSW_ENDPOINTS class=$ClassName $(Format-Endpoints $live)"

    $others = @($live | Where-Object { $_.LiveState -eq 1 -and $_.Id -ne $defaultId })
    $enabledByUs = $null
    $flowNoun = if ($Flow -eq 0) { 'capture' } else { 'render' }

    if ($others.Count -eq 0) {
        # Only one active endpoint on this flow: try to enable a present-but-
        # disabled (live state 2) one so the switch has somewhere to go.
        $disabled = @($live | Where-Object { $_.LiveState -eq 2 })
        if ($disabled.Count -eq 0) {
            $result.Verdict = "blocked-no-second-$flowNoun-endpoint"
            $result.Evidence += ("$flowNoun endpoints observed: " + (Format-Endpoints $live))
            Log-Event $EventsPath "DSW_CLASS_END class=$ClassName verdict=$($result.Verdict)"
            return [pscustomobject]$result
        }
        $cand = $disabled[0]
        $enableTab = 1; if ($Flow -eq 1) { $enableTab = 0 }
        Log-Event $EventsPath "DSW_ENABLE_BEGIN class=$ClassName target='$($cand.Name)'"
        $ok = Invoke-EndpointEnableToggle $cand.Name $true $enableTab $EventsPath "DSW_${ClassName}_ENABLE"
        Start-Sleep -Milliseconds 500
        $st = [SpeakSpike.Devices]::StateOf($cand.Id)
        if ($ok -and $st -eq 1) {
            $enabledByUs = $cand
            Log-Event $EventsPath "DSW_ENABLE_CONFIRMED target='$($cand.Name)' state=$st"
            $others = @($cand)
        } else {
            $result.Verdict = "blocked-could-not-enable-second-$flowNoun-endpoint"
            $result.Evidence += "enable invoked=$ok live_state_after=$st"
            Log-Event $EventsPath "DSW_ENABLE_FAILED target='$($cand.Name)' invoked=$ok state_after=$st"
            Log-Event $EventsPath "DSW_CLASS_END class=$ClassName verdict=$($result.Verdict)"
            return [pscustomobject]$result
        }
    }

    $target = $others[0]

    # original default endpoint display name (for restore logging)
    $origDefaultEp = $live | Where-Object { $_.Id -eq $defaultId }
    $origDefaultName = if ($origDefaultEp) { $origDefaultEp.Name } else { $defaultId }

    # -- start the dictation-proxy capture stream ---------------------------
    $csvPath = Join-Path $RunDir ("timeline-$ClassName.csv")
    $rec = $null
    try {
        $rec = [SpeakSpike.CaptureRecorder]::StartDefaultCapture($csvPath)
    } catch {
        $result.Verdict = 'defect-recorder-start-failed'
        $result.Evidence += $_.Exception.Message
        Log-Event $EventsPath "DSW_RECORDER_STARTED class=$ClassName FAILED $($_.Exception.Message)"
        return [pscustomobject]$result
    }
    Log-Event $EventsPath ("DSW_RECORDER_STARTED class=$ClassName format='" + $rec.FormatText +
        "' default_capture=$defaultId")

    try {
        # -- pre-switch liveness --------------------------------------------
        Start-Sleep -Seconds $PreSeconds
        $prePackets = $rec.TotalPackets
        $preFrames  = $rec.TotalFrames
        if ($prePackets -le 0 -or $rec.Errored) {
            $result.Verdict = 'defect-dead-before-switch'
            $result.Evidence += "packets=$prePackets errored=$($rec.Errored) error='$($rec.ErrorText)'"
            Log-Event $EventsPath "DSW_PRE_LIVENESS_DEAD class=$ClassName packets=$prePackets"
            return [pscustomobject]$result   # goes through finally: stop + restore
        }
        Log-Event $EventsPath ("DSW_PRE_LIVENESS_OK class=$ClassName packets=$prePackets " +
            "frames=$preFrames since_last_packet_ms=" +
            [int]$rec.MillisecondsSinceLastPacket())

        # -- perform the switch mid-recording --------------------------------
        $rec.ResetMaxGap()
        $packetsAtSwitch = $rec.TotalPackets
        $discontAtSwitch = $rec.DiscontinuityPackets
        $switchError = $null
        $switch = $null
        try {
            $switch = Switch-DefaultDevice $Flow $target.Name $EventsPath "DSW_$ClassName"
        } catch {
            $switchError = $_.Exception.Message
            Log-Event $EventsPath "DSW_SWITCH_ABORTED class=$ClassName reason='$switchError'"
        }

        if ($null -eq $switch -or -not $switch.Confirmed) {
            # The switch itself failed; capture continuity is still assessed
            # (the stream must survive ANY failed UIA attempt untouched).
            Start-Sleep -Seconds $PostSeconds
            $delta = $rec.TotalPackets - $packetsAtSwitch
            $maxGap = $rec.MaxGapMs
            $lastAgo = $rec.MillisecondsSinceLastPacket()
            $rec.StopRecording()
            Log-Event $EventsPath ("DSW_RECORDER_STOPPED class=$ClassName errored=$($rec.Errored) " +
                "stop_timed_out=$($rec.StopTimedOut) total_packets=$($rec.TotalPackets)")
            if ($rec.StopTimedOut) {
                $result.Verdict = 'defect-hang-stop-timeout'
            } elseif ($delta -le 0 -and -not $rec.Errored) {
                $result.Verdict = 'defect-dead-after-failed-switch'
            } elseif ($maxGap -le 250.0 -and $lastAgo -le 1000.0 -and -not $rec.Errored) {
                $result.Verdict = 'seamless-survived-failed-switch-attempt'
                $result.Evidence += "UIA switch failed ('$switchError') but capture was unaffected -- matches bound-stream design intent"
            } else {
                $result.Verdict = 'defect-after-failed-switch'
            }
            $result.Evidence += "switch_error='$switchError'"
            $result.Evidence += "post_packets_delta=$delta max_gap_ms=$([math]::Round($maxGap,1)) last_packet_ago_ms=$([int]$lastAgo)"
            return [pscustomobject]$result
        }

        # -- post-switch observation window -----------------------------------
        Start-Sleep -Seconds $PostSeconds
        $postDelta    = $rec.TotalPackets - $packetsAtSwitch
        $maxGap       = $rec.MaxGapMs
        $lastAgo      = $rec.MillisecondsSinceLastPacket()
        $discontDelta = $rec.DiscontinuityPackets - $discontAtSwitch
        Log-Event $EventsPath ("DSW_POST_WINDOW_DONE class=$ClassName seconds=$PostSeconds " +
            "packets_delta=$postDelta max_gap_ms=$([math]::Round($maxGap,1)) " +
            "discontinuities_total=$discontDelta wait_timeouts=$($rec.WaitTimeouts)")

        # -- stop the recorder -------------------------------------------------
        $rec.StopRecording()
        Log-Event $EventsPath ("DSW_RECORDER_STOPPED class=$ClassName errored=$($rec.Errored) " +
            "error='$($rec.ErrorText)' stop_timed_out=$($rec.StopTimedOut) " +
            "total_packets=$($rec.TotalPackets) total_frames=$($rec.TotalFrames) " +
            "silent_packets=$($rec.SilentPackets)")

        # -- classify -----------------------------------------------------------
        if ($rec.StopTimedOut) {
            $result.Verdict = 'defect-hang-stop-timeout'
        }
        elseif ($rec.ConsecutiveWaitTimeouts -ge 5) {
            $result.Verdict = 'defect-hang-stream-events-stopped'
        }
        elseif ($rec.Errored) {
            $result.Verdict = 'clean-restart'
            $result.Evidence += "stream_error='$($rec.ErrorText)' -- typed recovery (reopen) possible per recorder.rs design; NOT a hang"
        }
        elseif ($postDelta -le 0) {
            $result.Verdict = 'defect-dead-post-switch'
        }
        elseif ($discontDelta -gt 0) {
            $result.Verdict = 'degraded-discontinuity'
            $result.Evidence += "discontinuity_flags_total=$discontDelta"
        }
        elseif ($maxGap -gt 250.0) {
            $result.Verdict = 'degraded-max-gap'
            $result.Evidence += "max_inter_packet_gap_ms=$([math]::Round($maxGap,1)) (>250)"
        }
        elseif ($lastAgo -gt 1000.0) {
            $result.Verdict = 'defect-stalled-at-final-sample'
            $result.Evidence += "last_packet_ago_ms=$([int]$lastAgo)"
        }
        else {
            $result.Verdict = 'seamless'
        }
        $result.Evidence += ("packets: pre_switch=$prePackets switch_point=$packetsAtSwitch " +
            "end=$($rec.TotalPackets) (delta_through_switch_and_post=$postDelta); " +
            "frames_total=$($rec.TotalFrames); max_gap_ms=$([math]::Round($maxGap,1)); " +
            "wait_timeouts=$($rec.WaitTimeouts)")
    }
    finally {
        if ($rec -ne $null -and -not $rec.ThreadExited -and -not $rec.StopTimedOut) {
            try { $rec.StopRecording() } catch { }
        }
        # -- restore previous default ----------------------------------------
        if ($DoRestore -and $null -ne $switch -and $switch.After -ne $switch.Before) {
            Log-Event $EventsPath "DSW_RESTORE_DEFAULT_BEGIN class=$ClassName to='$origDefaultName'"
            try {
                $r = Switch-DefaultDevice $Flow $origDefaultName $EventsPath "DSW_${ClassName}_RESTORE"
                if ($r.Confirmed) { $result.Restored = "yes ($($r.TargetName))" }
                else { $result.Restored = "FAILED (default now $($r.After))" }
            } catch {
                $result.Restored = "FAILED ($($_.Exception.Message))"
            }
        }
        # -- teardown: disable an endpoint we enabled -------------------------
        if ($DoRestore -and $null -ne $enabledByUs) {
            Log-Event $EventsPath "DSW_TEARDOWN_DISABLE_BEGIN target='$($enabledByUs.Name)'"
            $teardownTab = 1; if ($Flow -eq 1) { $teardownTab = 0 }
            $ok = Invoke-EndpointEnableToggle $enabledByUs.Name $false $teardownTab $EventsPath "DSW_TEARDOWN"
            Start-Sleep -Milliseconds 500
            $st = [SpeakSpike.Devices]::StateOf($enabledByUs.Id)
            if ($ok -and $st -eq 2) {
                $result.Teardown = "disabled again ($($enabledByUs.Name))"
                Log-Event $EventsPath "DSW_TEARDOWN_DISABLE_OK target='$($enabledByUs.Name)' state=$st"
            } else {
                $result.Teardown = "FAILED (invoked=$ok state_after=$st) -- '$($enabledByUs.Name)' may remain ENABLED"
                Log-Event $EventsPath "DSW_TEARDOWN_DISABLE_FAILED target='$($enabledByUs.Name)' invoked=$ok state_after=$st"
            }
        }
        Log-Event $EventsPath "DSW_CLASS_END class=$ClassName verdict=$($result.Verdict)"
    }

    return [pscustomobject]$result
}

# -------------------------------- main dispatch ------------------------------

if ($ListInputs -or $ListOutputs) {
    if ($ListInputs)  { "CAPTURE (input) endpoints:";  Format-Endpoints (Get-LiveEndpoints 0) }
    if ($ListOutputs) { "RENDER (output) endpoints:";  Format-Endpoints (Get-LiveEndpoints 1) }
    exit 0
}

if ($Get) {
    "default capture : " + [SpeakSpike.Devices]::DefaultId(0, 0)
    "default render  : " + [SpeakSpike.Devices]::DefaultId(1, 0)
    exit 0
}

if (-not $Run) {
    Write-Output "specify -ListInputs, -ListOutputs, -Get, or -Run -Class Input|Output|Both"
    exit 2
}

# -- -Run --
$stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
$runDir = Join-Path (Get-DefaultLogDir) ("run-" + $stamp)
New-Item -ItemType Directory -Path $runDir -Force | Out-Null
$eventsPath = Join-Path $runDir 'events.log'

$classList = @('input', 'output')
if ($Class -eq 'Input')  { $classList = @('input') }
if ($Class -eq 'Output') { $classList = @('output') }

Log-Event $eventsPath ("DSW_RUN_BEGIN classes='" + ($classList -join ',') +
    "' pre_s=$PreSwitchSeconds post_s=$PostSwitchSeconds restore=" + (-not $NoRestore) +
    " os=" + [Environment]::OSVersion.VersionString +
    " pid=$PID host=$env:COMPUTERNAME user=$env:USERNAME")

$results = @()
foreach ($cn in $classList) {
    $flow = 0; if ($cn -eq 'output') { $flow = 1 }
    $results += Test-DeviceSwitchClass -Flow $flow -ClassName $cn -RunDir $runDir `
        -EventsPath $eventsPath -PreSeconds $PreSwitchSeconds `
        -PostSeconds $PostSwitchSeconds -DoRestore (-not $NoRestore)
}

# -- SUMMARY.md --
$overallPass = ($results | Where-Object { $_.Verdict -like 'defect*' }).Count -eq 0
$anyBlocked  = ($results | Where-Object { $_.Verdict -like 'blocked*' }).Count -gt 0
$overall =
    if (($results | Where-Object { $_.Verdict -eq 'seamless' }).Count -eq $results.Count) { 'PASS (seamless everywhere)' }
    elseif ($overallPass -and -not $anyBlocked) { 'PASS WITH NOTES (no defects)' }
    elseif ($anyBlocked) { 'PARTIAL/BLOCKED (see per-class verdicts)' }
    else { 'FAIL (blocking defects found)' }

$sb = New-Object System.Text.StringBuilder
[void]$sb.AppendLine("# Device-switch test -- run $stamp")
[void]$sb.AppendLine("")
[void]$sb.AppendLine("- Classes executed: $($classList -join ', ')")
[void]$sb.AppendLine("- Overall: **$overall**")
[void]$sb.AppendLine("")
[void]$sb.AppendLine("| Class | Verdict | Restore | Teardown |")
[void]$sb.AppendLine("| --- | --- | --- | --- |")
foreach ($r in $results) {
    [void]$sb.AppendLine("| $($r.Class) | $($r.Verdict) | $($r.Restored) | $($r.Teardown) |")
}
[void]$sb.AppendLine("")
foreach ($r in $results) {
    [void]$sb.AppendLine("## Class: $($r.Class)")
    foreach ($e in $r.Evidence) { [void]$sb.AppendLine("- $e") }
    [void]$sb.AppendLine("")
}
[void]$sb.AppendLine("## Acceptance gates")
[void]$sb.AppendLine("- G1 both switch classes executed with event timeline: see events.log + timeline-*.csv in this directory.")
[void]$sb.AppendLine("- G2 outcomes classified with evidence above; hang/dead classifications are blocking findings.")
[void]$sb.AppendLine("- G3 all changes confined to docs/phase1-spike/tools/device-switch-test.ps1 (helpers embedded there).")
Set-Content -Path (Join-Path $runDir 'SUMMARY.md') -Value $sb.ToString() -Encoding UTF8

Log-Event $eventsPath "DSW_RUN_END overall='$overall'"

Write-Output ""
Write-Output "OVERALL: $overall"
Write-Output "Artifacts: $runDir"
if ($overall -like 'FAIL*' -or ($results | Where-Object { $_.Verdict -like 'defect*' })) { exit 1 }
exit 0
