# Reads or changes the Windows DEFAULT audio OUTPUT endpoint without elevation,
# using the undocumented-but-widely-used PolicyConfig COM interface (same
# mechanism as SoundSwitch/AudioSwitcher).
#
# Modes:
#   -List            print render endpoints (MMDevice IDs + friendly names)
#   -Get             print the current default render endpoint (console role)
#   -Set -Name X     make the first render endpoint whose friendly name contains
#                    X (case-insensitive) the default for console/multimedia/
#                    communication roles
#   -RestoreTo X     same as -Set (kept as explicit revert marker)
#
# Usage:
#   pwsh -File set-default-output.ps1 -List
#   pwsh -File set-default-output.ps1 -Get
#   pwsh -File set-default-output.ps1 -Set -Name "Realtek"
#   pwsh -File set-default-output.ps1 -RestoreTo "Focusrite"
param(
    [switch]$List,
    [switch]$Get,
    [switch]$Set,
    [string]$Name,
    [string]$RestoreTo
)

$ErrorActionPreference = 'Stop'

$cs = @'
using System;
using System.Runtime.InteropServices;

namespace SpeakSpike {
    [StructLayout(LayoutKind.Sequential)]
    public struct PropertyKey {
        public Guid fmtid;
        public uint pid;
    }

    [StructLayout(LayoutKind.Explicit)]
    public struct PropVariant {
        [FieldOffset(0)] public ushort vt;
        [FieldOffset(8)] public IntPtr pointerValue;
    }

    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
    class MMDeviceEnumeratorCom { }

    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IMMDeviceEnumerator {
        int EnumAudioEndpoints(int dataFlow, int stateMask, out IntPtr devices);
        [PreserveSig]
        int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice endpoint);
        int GetDevice([MarshalAs(UnmanagedType.LPWStr)] string id, out IMMDevice device);
        // remaining methods unused
        int RegisterEndpointNotificationCallback(IntPtr cb);
        int UnregisterEndpointNotificationCallback(IntPtr cb);
    }

    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IMMDevice {
        int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, out IntPtr iface);
        int OpenPropertyStore(int access, out IntPtr store);
        int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
        int GetState(out int state);
    }

    // IPolicyConfig (IID F8679F50-850A-41CF-9C72-430F290290C8).
    // Only the vtable positions matter; unused methods are stubbed with
    // pointer-sized arguments to preserve layout.
    [ComImport, Guid("F8679F50-850A-41CF-9C72-430F290290C8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IPolicyConfig {
        int GetMixFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr ppFormat);
        int GetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int defaultFormat, IntPtr ppFormat);
        int ResetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId);
        int SetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr endpointFormat, IntPtr mixFormat);
        int GetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int defaultPeriod, IntPtr paDefaultPeriod, IntPtr paminPeriod);
        int SetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr period);
        int GetShareMode([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr pdwMode);
        int SetShareMode([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr mode);
        int GetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int dataFlow, ref PropertyKey key, out PropVariant value);
        int SetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int dataFlow, ref PropertyKey key, ref PropVariant value);
        int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int role);
        int SetEndpointVisibility([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int visible);
    }

    [ComImport, Guid("294935CE-F637-4E7C-A41B-AB255460B862")]
    class PolicyConfigClientCom { }

    // Same vtable layout as IPolicyConfig, but the IID exposed by modern
    // Windows 10/11 builds.
    [ComImport, Guid("CA286FC3-91FD-42C3-8E9B-CAAFA66242E3"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IPolicyConfigModern {
        int GetMixFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr ppFormat);
        int GetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int defaultFormat, IntPtr ppFormat);
        int ResetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId);
        int SetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr endpointFormat, IntPtr mixFormat);
        int GetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int defaultPeriod, IntPtr paDefaultPeriod, IntPtr paminPeriod);
        int SetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr period);
        int GetShareMode([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr pdwMode);
        int SetShareMode([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr mode);
        int GetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int dataFlow, ref PropertyKey key, out PropVariant value);
        int SetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int dataFlow, ref PropertyKey key, ref PropVariant value);
        int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int role);
        int SetEndpointVisibility([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int visible);
    }

    public static class DefaultDevice {
        // PKEY_Device_FriendlyName
        static PropertyKey nameKey = new PropertyKey {
            fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"), pid = 14
        };

        static IPolicyConfig CreatePolicyConfig() {
            // Modern CLSID first, legacy fallback.
            try {
                var t = Type.GetTypeFromCLSID(new Guid("294935CE-F637-4E7C-A41B-AB255460B862"));
                return (IPolicyConfig)Activator.CreateInstance(t);
            } catch { }
            var t2 = Type.GetTypeFromCLSID(new Guid("C2F03A33-21F5-47FA-B4BB-156362A2F239"));
            return (IPolicyConfig)Activator.CreateInstance(t2);
        }

        public static IMMDeviceEnumerator Enumerator() {
            return (IMMDeviceEnumerator)(object)new MMDeviceEnumeratorCom();
        }

        public static string DefaultRenderId() {
            var en = Enumerator();
            IMMDevice dev;
            int hr = en.GetDefaultAudioEndpoint(0 /*eRender*/, 0 /*eConsole*/, out dev);
            Marshal.ThrowExceptionForHR(hr);
            string id;
            dev.GetId(out id);
            return id;
        }

        // Returns (friendlyName, active) for one render endpoint registry key
        // name (including braces, e.g. "{5a57e8a4-...}").
        static void EndpointInfo(string keyName, out string name, out int deviceState) {
            var basePath = @"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render";
            using (var root = Microsoft.Win32.Registry.LocalMachine.OpenSubKey(basePath + "\\" + keyName)) {
                if (root == null) { deviceState = 8; name = null; return; }
                object st = root.GetValue("DeviceState");
                deviceState = st is int ? (int)st : 8;
            }
            name = null;
            using (var props = Microsoft.Win32.Registry.LocalMachine.OpenSubKey(
                       basePath + "\\" + keyName + "\\Properties")) {
                if (props == null) return;
                string fallback = null;
                foreach (var valName in props.GetValueNames()) {
                    var lower = valName.ToLowerInvariant();
                    if (lower.StartsWith("{a45c254e-df1c-4efd-8020-67d146a850e0},")) {
                        var val = Convert.ToString(props.GetValue(valName),
                            System.Globalization.CultureInfo.InvariantCulture);
                        if (lower.EndsWith(",14")) { name = val; break; }
                        if (lower.EndsWith(",2")) fallback = val;
                    }
                }
                if (name == null) name = fallback;
            }
        }

        public static string IdToFriendlyName(string id) {
            string keyName = id.Substring(id.LastIndexOf('.') + 1);
            string name; int state;
            EndpointInfo(keyName, out name, out state);
            return name ?? id;
        }

        public static void ListRenderEndpoints() {
            string defId = DefaultRenderId();
            var basePath = @"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render";
            using (var keys = Microsoft.Win32.Registry.LocalMachine.OpenSubKey(basePath)) {
                foreach (var sub in keys.GetSubKeyNames()) {
                    string name; int state;
                    EndpointInfo(sub, out name, out state);
                    if (state != 1 || string.IsNullOrEmpty(name)) continue; // 1 = DEVICE_STATE_ACTIVE
                    var id = "{0.0.0.00000000}." + sub;
                    Console.WriteLine("{0}  {1}{2}", id, name,
                        id.Equals(defId, StringComparison.OrdinalIgnoreCase) ? "  (default)" : "");
                }
            }
        }

        static void SetAllRoles(object cfgObj, string deviceId) {
            // Modern IID first; legacy fallback for older builds.
            try {
                var modern = (IPolicyConfigModern)cfgObj;
                for (int role = 0; role <= 2; role++)
                    Marshal.ThrowExceptionForHR(modern.SetDefaultEndpoint(deviceId, role));
                return;
            } catch (InvalidCastException) { }
            var legacy = (IPolicyConfig)cfgObj;
            for (int role = 0; role <= 2; role++)
                Marshal.ThrowExceptionForHR(legacy.SetDefaultEndpoint(deviceId, role));
        }

        public static void SetDefaultByName(string needle) {
            var t = Type.GetTypeFromCLSID(new Guid("294935CE-F637-4E7C-A41B-AB255460B862"));
            object cfg = Activator.CreateInstance(t);
            string matchId = null;
            string matchName = null;
            var basePath = @"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render";
            using (var keys = Microsoft.Win32.Registry.LocalMachine.OpenSubKey(basePath)) {
                foreach (var sub in keys.GetSubKeyNames()) {
                    string name; int state;
                    EndpointInfo(sub, out name, out state);
                    if (state != 1 || string.IsNullOrEmpty(name)) continue;
                    if (name.ToLowerInvariant().Contains(needle.ToLowerInvariant())) {
                        matchId = "{0.0.0.00000000}." + sub;
                        matchName = name;
                        break;
                    }
                }
            }
            if (matchId == null)
                throw new InvalidOperationException("no render endpoint matching '" + needle + "'");
            SetAllRoles(cfg, matchId);
            Console.WriteLine("default render endpoint set to '{0}' ({1})", matchName, matchId);
        }
    }
}
'@

Add-Type -TypeDefinition $cs -Language CSharp

if ($List) { [SpeakSpike.DefaultDevice]::ListRenderEndpoints(); exit 0 }
if ($Get)  {
    $id = [SpeakSpike.DefaultDevice]::DefaultRenderId()
    Write-Output "$id  $([SpeakSpike.DefaultDevice]::IdToFriendlyName($id))"
    exit 0
}
$target = if ($Set) { $Name } elseif ($RestoreTo) { $RestoreTo } else { $null }
if ($target) { [SpeakSpike.DefaultDevice]::SetDefaultByName($target); exit 0 }

Write-Error "specify -List, -Get, -Set -Name <x> or -RestoreTo <x>"
