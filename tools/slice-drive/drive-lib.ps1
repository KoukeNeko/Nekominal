# Dot-source this file.  Drives the spike app with real SendInput (mouse + keyboard with scan codes) and
# captures the window.  Input is ONLY sent while the foreground window belongs to the started process.
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
public static class D {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetCursorInfo(ref CURSORINFO ci);
  [DllImport("user32.dll")] public static extern IntPtr LoadCursor(IntPtr inst, IntPtr name);
  [DllImport("user32.dll")] public static extern IntPtr GetKeyboardLayout(uint thread);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct CURSORINFO { public int cbSize; public int flags; public IntPtr hCursor; public POINT pt; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Explicit, Size = 40)] public struct INPUT {
    [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mi; [FieldOffset(8)] public KEYBDINPUT ki; }
  [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  public static void Key(ushort vk, bool up) {
    uint sc = MapVirtualKey(vk, 0);
    bool ext = (vk >= 0x21 && vk <= 0x2E) || vk == 0x6F || vk == 0x90 || vk == 0xA3 || vk == 0xA5 || vk == 0x5B || vk == 0x5C;
    INPUT[] i = new INPUT[1]; i[0].type = 1; i[0].ki.wVk = vk; i[0].ki.wScan = (ushort)sc;
    i[0].ki.dwFlags = 0x0008 /*SCANCODE*/ | (up ? 0x0002u : 0u) | (ext ? 0x0001u : 0u);
    // KEYEVENTF_SCANCODE with wVk set: the system derives the vk from the scan code of the active layout; keep wVk too.
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void KeyVk(ushort vk, bool up) {   // virtual-key only (what many automation tools send)
    INPUT[] i = new INPUT[1]; i[0].type = 1; i[0].ki.wVk = vk; i[0].ki.dwFlags = up ? 0x0002u : 0u;
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void Unicode(char c) {
    INPUT[] i = new INPUT[2];
    i[0].type = 1; i[0].ki.wScan = c; i[0].ki.dwFlags = 0x0004;
    i[1].type = 1; i[1].ki.wScan = c; i[1].ki.dwFlags = 0x0004 | 0x0002;
    SendInput(2, i, Marshal.SizeOf(typeof(INPUT)));
  }
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  // Absolute move as a real input event (SetCursorPos alone does not produce an ordered WM_MOUSEMOVE).
  public static void MoveAbs(int x, int y) {
    int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77), vw = GetSystemMetrics(78), vh = GetSystemMetrics(79);
    INPUT[] i = new INPUT[1]; i[0].type = 0;
    i[0].mi.dx = (int)(((long)(x - vx) * 65535) / (vw - 1)); i[0].mi.dy = (int)(((long)(y - vy) * 65535) / (vh - 1));
    i[0].mi.dwFlags = 0x0001 | 0x8000 | 0x4000; // MOVE | ABSOLUTE | VIRTUALDESK
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void Mouse(uint flags, uint data) {
    INPUT[] i = new INPUT[1]; i[0].type = 0; i[0].mi.dwFlags = flags; i[0].mi.mouseData = data;
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static uint ForegroundPid() { uint p; GetWindowThreadProcessId(GetForegroundWindow(), out p); return p; }
  public static IntPtr FindWindow(uint pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) { RECT r; GetWindowRect(h, out r); if (r.Right - r.Left > 300 && r.Bottom - r.Top > 200) found = h; } return true; }, IntPtr.Zero);
    return found;
  }
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  // Foreground rights without sending any key: temporarily share input state with the current foreground thread.
  public static void ForceForeground(IntPtr h) {
    IntPtr fg = GetForegroundWindow(); uint fgt = GetWindowThreadProcessId(fg, IntPtr.Zero); uint me = GetCurrentThreadId();
    if (fgt != me) AttachThreadInput(me, fgt, true);
    BringWindowToTop(h); SetForegroundWindow(h);
    if (fgt != me) AttachThreadInput(me, fgt, false);
  }
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
  public static uint PidAt(int x, int y) { POINT p; p.X = x; p.Y = y; IntPtr h = GetAncestor(WindowFromPoint(p), 2); uint pid; GetWindowThreadProcessId(h, out pid); return pid; }
  public static List<IntPtr> AllWindows(uint pid) {
    List<IntPtr> res = new List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) { RECT r; GetWindowRect(h, out r); if (r.Right - r.Left > 300 && r.Bottom - r.Top > 200) res.Add(h); } return true; }, IntPtr.Zero);
    return res;
  }
  public static IntPtr CursorHandle() { CURSORINFO ci = new CURSORINFO(); ci.cbSize = Marshal.SizeOf(typeof(CURSORINFO)); GetCursorInfo(ref ci); return ci.hCursor; }
}
'@
[void][D]::SetProcessDpiAwarenessContext([IntPtr](-4))

$script:App = $null
$script:Hwnd = [IntPtr]::Zero
$script:LayoutFile = $null

function Start-App([string]$Exe, [string]$Log, [string]$Layout, [string]$ExtraArgs = '', [hashtable]$Env = @{}, [int]$SettleMs = 3000) {
    if ($Log) { Remove-Item $Log -ErrorAction SilentlyContinue }
    if ($Layout) { Remove-Item $Layout -ErrorAction SilentlyContinue }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe; $psi.Arguments = $ExtraArgs; $psi.UseShellExecute = $false; $psi.WorkingDirectory = Split-Path $Exe
    # The Claude Code host exports NO_COLOR and SetEnvironmentVariable($null) leaves an empty entry behind in the
    # snapshot ProcessStartInfo takes: drop it explicitly, or pwsh strips every ANSI sequence.
    [void]$psi.Environment.Remove('NO_COLOR')
    # Nothing of the host that drives the test (an agent session) reaches the app or the shells in it: $Env below
    # puts back what a scenario asks for.
    foreach ($k in @($psi.Environment.Keys | Where-Object { $_ -match '^(CLAUDE|ANTHROPIC|MCP_)' })) { [void]$psi.Environment.Remove($k) }
    if ($Log) { $psi.Environment['SPIKE_LOG'] = $Log }
    if ($Layout) { $psi.Environment['SPIKE_LAYOUT'] = $Layout }
    # A $null value removes the variable from the child's environment.
    foreach ($k in $Env.Keys) { if ($null -eq $Env[$k]) { [void]$psi.Environment.Remove($k) } else { $psi.Environment[$k] = [string]$Env[$k] } }
    $script:App = [Diagnostics.Process]::Start($psi)
    $script:LayoutFile = $Layout
    $script:Hwnd = [IntPtr]::Zero
    for ($i = 0; $i -lt 600 -and $script:Hwnd -eq [IntPtr]::Zero; $i++) {
        if ($script:App.HasExited) { throw "app exited early code $($script:App.ExitCode)" }
        Start-Sleep -Milliseconds 25; $script:Hwnd = [D]::FindWindow([uint32]$script:App.Id)
    }
    if ($script:Hwnd -eq [IntPtr]::Zero) { throw 'no window' }
    Start-Sleep -Milliseconds $SettleMs
    [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 400
}

function Stop-App { if ($script:App -and -not $script:App.HasExited) { [void]$script:App.CloseMainWindow(); if (-not $script:App.WaitForExit(3000)) { $script:App.Kill() } } }

function Assert-Foreground {
    # Other test apps may share this desktop: only take the foreground from an idle host (the Claude app / a shell);
    # wait (up to 120 s) while somebody else's window is active, and never send input to a window that is not ours.
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    while ([D]::ForegroundPid() -ne [uint32]$script:App.Id) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "foreground window is not the test app (pid $([D]::ForegroundPid())); nothing sent" }
        $name = (Get-Process -Id ([D]::ForegroundPid()) -ErrorAction SilentlyContinue).ProcessName
        if ($name -in 'claude', 'pwsh', 'powershell', 'WindowsTerminal', 'conhost', 'explorer', 'ShellExperienceHost', 'SearchHost' -or -not $name) {
            [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 300
        } else { Start-Sleep -Milliseconds 500 }
    }
}
function Win-Rect {
    if ([D]::IsIconic($script:Hwnd)) { [void][D]::ShowWindow($script:Hwnd, 9); Start-Sleep -Milliseconds 400 }
    $r = New-Object D+RECT
    if ([D]::DwmGetWindowAttribute($script:Hwnd, 9, [ref]$r, 16) -ne 0) { [void][D]::GetWindowRect($script:Hwnd, [ref]$r) }
    $r
}

# Physical pixels per DIP of the app window (1.5 at 150 %), from the DPI of the window itself.
function Get-Scale { [double][D]::GetDpiForWindow($script:Hwnd) / 96.0 }

function Get-Layout {
    $h = @{}
    if ($script:LayoutFile -and (Test-Path $script:LayoutFile)) {
        foreach ($l in Get-Content $script:LayoutFile) { $p = $l -split ' '; if ($p.Count -ge 5) { $h[$p[0]] = @{ X = [int]$p[1]; Y = [int]$p[2]; W = [int]$p[3]; H = [int]$p[4] } } }
    }
    $h
}

function Refresh-Layout { Press 0x7B; Start-Sleep -Milliseconds 400 }   # F12: app rewrites the layout file

function Move-To([int]$x, [int]$y) { [D]::MoveAbs($x, $y) }

function Click-At([int]$x, [int]$y, [string]$Button = 'left', [int]$Gap = 400) {
    Assert-Foreground
    if ([D]::PidAt($x, $y) -ne [uint32]$script:App.Id) { throw "window under ($x,$y) belongs to pid $([D]::PidAt($x, $y)), not the test app; click skipped" }
    Move-To $x $y; Start-Sleep -Milliseconds 120
    Move-To ($x + 1) $y; Start-Sleep -Milliseconds 40; Move-To $x $y; Start-Sleep -Milliseconds 60
    if ($Button -eq 'left') { [D]::Mouse(0x0002, 0); Start-Sleep -Milliseconds 50; [D]::Mouse(0x0004, 0) }
    elseif ($Button -eq 'right') { [D]::Mouse(0x0008, 0); Start-Sleep -Milliseconds 50; [D]::Mouse(0x0010, 0) }
    elseif ($Button -eq 'middle') { [D]::Mouse(0x0020, 0); Start-Sleep -Milliseconds 50; [D]::Mouse(0x0040, 0) }
    Start-Sleep -Milliseconds $Gap
}

function Click-El([string]$Name, [double]$Fx = 0.5, [double]$Fy = 0.5, [string]$Button = 'left', [int]$Gap = 400) {
    $l = Get-Layout
    if (-not $l.ContainsKey($Name)) { throw "no layout entry '$Name' (have: $($l.Keys -join ', '))" }
    $e = $l[$Name]
    Click-At ([int]($e.X + $e.W * $Fx)) ([int]($e.Y + $e.H * $Fy)) $Button $Gap
}

function Drag-To([int]$x1, [int]$y1, [int]$x2, [int]$y2, [int]$Steps = 30, [int]$StepMs = 20) {
    Assert-Foreground
    Move-To $x1 $y1; Start-Sleep -Milliseconds 200
    [D]::Mouse(0x0002, 0); Start-Sleep -Milliseconds 200
    for ($i = 1; $i -le $Steps; $i++) { Move-To ([int]($x1 + ($x2 - $x1) * $i / $Steps)) ([int]($y1 + ($y2 - $y1) * $i / $Steps)); Start-Sleep -Milliseconds $StepMs }
    Start-Sleep -Milliseconds 200
    [D]::Mouse(0x0004, 0); Start-Sleep -Milliseconds 600
}

# Press a key (virtual-key code) with modifiers (vk codes: 0x11 ctrl, 0x10 shift, 0x12 alt); scan codes are included.
function Press([int]$Vk, [int[]]$Mods = @(), [int]$HoldMs = 40, [int]$GapMs = 150) {
    Assert-Foreground
    foreach ($m in $Mods) { [D]::Key([uint16]$m, $false); Start-Sleep -Milliseconds 15 }
    [D]::Key([uint16]$Vk, $false); Start-Sleep -Milliseconds $HoldMs; [D]::Key([uint16]$Vk, $true)
    $r = $Mods.Clone(); [array]::Reverse($r)
    foreach ($m in $r) { Start-Sleep -Milliseconds 15; [D]::Key([uint16]$m, $true) }
    Start-Sleep -Milliseconds $GapMs
}

function Type-Unicode([string]$Text, [int]$GapMs = 80) { Assert-Foreground; foreach ($c in $Text.ToCharArray()) { [D]::Unicode($c); Start-Sleep -Milliseconds $GapMs } }

function Shot([string]$Path, [int]$MarginTop = 0, [int]$MarginBottom = 0) {
    # Only the window rectangle is captured (margins are ignored on purpose: nothing of the rest of the desktop is recorded).
    $r = Win-Rect
    $w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $g.Dispose(); $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    "shot $Path ${w}x${h}"
}

function ShotRegion([string]$Path, [int]$x, [int]$y, [int]$w, [int]$h) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $g.Dispose(); $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    "shot $Path ${w}x${h}"
}

function Log-Tail([string]$Log, [int]$N = 25) { if (Test-Path $Log) { Get-Content $Log -Tail $N } }










function Shot-Window([IntPtr]$h, [string]$Path) {
    $r = New-Object D+RECT
    if ([D]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) -ne 0) { [void][D]::GetWindowRect($h, [ref]$r) }
    $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size $w, $hh))
    $g.Dispose(); $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    "shot $Path ${w}x${hh} at $($r.Left),$($r.Top)"
}

