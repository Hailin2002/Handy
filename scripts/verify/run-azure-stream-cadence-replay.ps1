param(
    [string]$WavPath,

    [ValidateSet(30, 120)]
    [int]$ChunkMs = 30,

    [int]$SegmentationSilenceMs = 300,

    [int]$StablePartialThreshold = 1,

    [int]$MaxAudioMs = 0,

    [string]$MeetingAuditId,

    [string]$AuditOutputPath,

    [string]$AuditFilter,

    [switch]$AuditIncludeText,

    [switch]$Fast
)

$ErrorActionPreference = 'Stop'

if (-not ('SpeakCredentialReader' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class SpeakCredentialReader
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NativeCredential
    {
        public UInt32 Flags;
        public UInt32 Type;
        public string TargetName;
        public string Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public UInt32 CredentialBlobSize;
        public IntPtr CredentialBlob;
        public UInt32 Persist;
        public UInt32 AttributeCount;
        public IntPtr Attributes;
        public string TargetAlias;
        public string UserName;
    }

    [DllImport("Advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredRead(string target, UInt32 type, UInt32 flags, out IntPtr credential);

    [DllImport("Advapi32.dll", SetLastError = true)]
    private static extern void CredFree(IntPtr credential);

    public static byte[] ReadBlob(string target)
    {
        IntPtr pointer;
        if (!CredRead(target, 1, 0, out pointer))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        try
        {
            NativeCredential credential = Marshal.PtrToStructure<NativeCredential>(pointer);
            byte[] bytes = new byte[credential.CredentialBlobSize];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            return bytes;
        }
        finally
        {
            CredFree(pointer);
        }
    }
}
'@
}

$blob = [SpeakCredentialReader]::ReadBlob('speak:speech-pipeline/azure.speak')
$key = if ($blob.Length -ge 2 -and ($blob | Where-Object { $_ -eq 0 }).Count -gt ($blob.Length / 4)) {
    [Text.Encoding]::Unicode.GetString($blob).TrimEnd([char]0)
} else {
    [Text.Encoding]::UTF8.GetString($blob).TrimEnd([char]0)
}

try {
    $env:SPEAK_AZURE_KEY = $key
    $env:SPEAK_AZURE_REGION = 'germanywestcentral'
    $env:SPEAK_AZURE_ENDPOINT = 'https://germanywestcentral.api.cognitive.microsoft.com'
    $env:SPEAK_AZURE_LANGUAGE = 'de-DE'
    if ($MeetingAuditId) {
        if (-not $AuditOutputPath) {
            throw 'AuditOutputPath is required with MeetingAuditId'
        }
        if ($AuditIncludeText) {
            $env:SPEAK_AZURE_AUDIT_INCLUDE_TEXT = '1'
        }
        node "$PSScriptRoot\audit-meeting-azure-reference.mjs" $MeetingAuditId $AuditOutputPath $AuditFilter
    } elseif ($Fast) {
        if (-not $WavPath) { throw 'WavPath is required with Fast' }
        node "$PSScriptRoot\azure-fast-transcription-replay.mjs" $WavPath
    } else {
        if (-not $WavPath) { throw 'WavPath is required' }
        node "$PSScriptRoot\azure-stream-cadence-replay.mjs" $WavPath $ChunkMs $SegmentationSilenceMs $StablePartialThreshold $MaxAudioMs
    }
    if ($LASTEXITCODE -ne 0) {
        throw "Azure cadence replay exited with code $LASTEXITCODE"
    }
} finally {
    Remove-Item Env:SPEAK_AZURE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:SPEAK_AZURE_REGION -ErrorAction SilentlyContinue
    Remove-Item Env:SPEAK_AZURE_ENDPOINT -ErrorAction SilentlyContinue
    Remove-Item Env:SPEAK_AZURE_LANGUAGE -ErrorAction SilentlyContinue
    Remove-Item Env:SPEAK_AZURE_AUDIT_INCLUDE_TEXT -ErrorAction SilentlyContinue
    $key = $null
    [Array]::Clear($blob, 0, $blob.Length)
}
