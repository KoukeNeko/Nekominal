# Read-only witness for "the app made no sound": polls the peak meter of the default audio render endpoint
# (IAudioMeterInformation) and keeps the largest value seen. Dot-source it; it plays nothing by itself.
#
#   Start-SoundWitness                   opens the meter and starts a 5 ms polling thread (returns the baseline peak)
#   Reset-SoundWitness                   forgets the peaks seen so far
#   Get-SoundPeak                        the largest endpoint peak since the last reset (0..1)
#   Measure-Peak { ... } [-WindowMs 1500]  runs the block and returns the largest peak up to WindowMs after it ($null: baseline not quiet)
#   Test-WitnessPositive                 plays ONE MessageBeep and reports whether the meter saw it (calibration, audible)
#
# The meter sees every sound any process renders to the default endpoint, so the baseline must be quiet; a
# nonzero baseline makes Measure-Peak return $null (inconclusive) instead of a pass.

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;

public static class SoundWitness {
  [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class Enumerator {}
  [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IEnumerator {
    int EnumAudioEndpoints(int dataFlow, int stateMask, out IntPtr devices);
    int GetDefaultAudioEndpoint(int dataFlow, int role, out IDevice device);
  }
  [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IDevice {
    int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
  }
  [ComImport, Guid("C02216F6-8C67-4B5B-9D00-D008E73E0064"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IMeter {
    int GetPeakValue(out float peak);
  }
  [DllImport("user32.dll")] public static extern bool MessageBeep(uint type);

  static IMeter meter;
  static Thread poller;
  static volatile bool stop;
  static float peak;
  static readonly object gate = new object();

  public static float Open() {
    var enumerator = (IEnumerator)new Enumerator();
    IDevice device;
    Marshal.ThrowExceptionForHR(enumerator.GetDefaultAudioEndpoint(0 /*eRender*/, 1 /*eMultimedia*/, out device));
    var iid = new Guid("C02216F6-8C67-4B5B-9D00-D008E73E0064");
    object meterObject;
    Marshal.ThrowExceptionForHR(device.Activate(ref iid, 23 /*CLSCTX_ALL*/, IntPtr.Zero, out meterObject));
    meter = (IMeter)meterObject;
    stop = false;
    poller = new Thread(() => {
      while (!stop) {
        float now;
        if (meter.GetPeakValue(out now) == 0) { lock (gate) { if (now > peak) peak = now; } }
        Thread.Sleep(5);
      }
    });
    poller.IsBackground = true;
    poller.Start();
    Thread.Sleep(200);
    return Take();
  }

  public static float Take() { lock (gate) { float p = peak; peak = 0; return p; } }
  public static float Peek() { lock (gate) { return peak; } }
  public static void Close() { stop = true; if (poller != null) poller.Join(); }
}
'@

# Peaks at or above this are sound; the idle noise floor of the meter is about 1e-34.
$global:WitnessThreshold = 1e-4

function Start-SoundWitness {
    $baseline = [SoundWitness]::Open()
    'witness open, baseline peak {0:E2}' -f $baseline
}

function Reset-SoundWitness { [void][SoundWitness]::Take() }

function Get-SoundPeak { [SoundWitness]::Peek() }

# Runs the block and returns the largest peak seen up to WindowMs after it, or $null when the baseline before it
# was already loud (somebody else is playing: the row is inconclusive, not a pass).
function Measure-Peak([scriptblock]$Action, [int]$WindowMs = 1500) {
    Start-Sleep -Milliseconds 300
    $baseline = [SoundWitness]::Take()
    Start-Sleep -Milliseconds 700
    $baseline = [Math]::Max($baseline, [SoundWitness]::Take())
    if ($baseline -ge $global:WitnessThreshold) { Write-Warning ('inconclusive: baseline peak {0:E2}' -f $baseline); return $null }
    & $Action
    Start-Sleep -Milliseconds $WindowMs
    [SoundWitness]::Take()
}

function Test-WitnessPositive {
    [void][SoundWitness]::Take(); Start-Sleep -Milliseconds 700; [void][SoundWitness]::Take()
    [void][SoundWitness]::MessageBeep(0)
    Start-Sleep -Milliseconds 1200
    $peak = [SoundWitness]::Take()
    'positive control (one MessageBeep): peak {0:E2}, seen: {1}' -f $peak, ($peak -ge $global:WitnessThreshold)
}