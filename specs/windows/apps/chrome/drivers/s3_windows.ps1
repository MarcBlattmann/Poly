# What windows does Chrome's S3 run create, and in which order do they appear? (spec-chrome-win-s3)
#
# Starts Chrome as S3 does (s3_driver.ps1), watches every window of every chrome.exe process from outside
# (out-of-context WinEvent hook, no injection, nothing of Chrome read) and dumps, at fixed steps, every window
# of those processes: class, styles, owner, parent, rectangles, state, DWM and layered attributes. The steps:
#   a  the first window with the page's title is visible
#   b  after a mouse move, one click and the keys p o l y (SendInput)
#   c  after SetWindowPos to another size (resize)
#   d  after SW_MINIMIZE and SW_RESTORE of the main window
#   e  WM_CLOSE (events until the last chrome.exe is gone)
# Handles are printed as w1, w2, ... (order of first sight); threads as t1, t2, ...; times in milliseconds
# from the start of the run. Needs the interactive desktop (see interactive_task.ps1).
param(
  [Parameter(Mandatory)][string]$ChromeDir,
  [Parameter(Mandatory)][string]$Out,
  [string]$Work = "$env:USERPROFILE\pc",
  [switch]$Gpu,
  [switch]$NoHook
)
$ErrorActionPreference = 'Stop'
$Page = 'file:///' + ($Work -replace '\\', '/') + '/pages/s3.html'
$Ud = "$Work\ud\s3win"
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class O {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
  [DllImport("user32.dll", EntryPoint="GetClassLongPtrW")] public static extern IntPtr GetClassLongPtr(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern int GetWindowRgnBox(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetLayeredWindowAttributes(IntPtr h, out uint key, out byte alpha, out uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(IntPtr h, out uint a);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] i, int size);
  [DllImport("user32.dll")] public static extern bool PeekMessageW(out MSG m, IntPtr h, uint a, uint b, uint remove);
  [DllImport("user32.dll")] public static extern IntPtr DispatchMessageW(ref MSG m);
  [DllImport("user32.dll")] public static extern bool GetWindowPlacement(IntPtr h, ref WINDOWPLACEMENT p);
  [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint tid, ref GUITHREADINFO i);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out int v, int size);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT v, int size);
  [DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] public static extern bool QueryFullProcessImageNameW(IntPtr h, uint flags, StringBuilder s, ref int n);
  public static Dictionary<uint, bool> IsChromeCache = new Dictionary<uint, bool>();
  public static bool IsChrome(uint pid, string cls) {
    bool b; if (IsChromeCache.TryGetValue(pid, out b)) return b;
    IntPtr h = OpenProcess(0x1000, false, pid);
    if (h == IntPtr.Zero) return cls.StartsWith("Chrome_");
    var s = new StringBuilder(520); int n = 520; bool ok = QueryFullProcessImageNameW(h, 0, s, ref n); CloseHandle(h);
    b = ok && s.ToString().EndsWith("chrome.exe", StringComparison.OrdinalIgnoreCase);
    IsChromeCache[pid] = b; return b;
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int x, y; }
  [StructLayout(LayoutKind.Sequential)] public struct WINDOWPLACEMENT { public uint length, flags, showCmd; public POINT ptMin, ptMax; public RECT normal; }
  [StructLayout(LayoutKind.Sequential)] public struct GUITHREADINFO { public uint size, flags; public IntPtr active, focus, capture, menuOwner, moveSize, caret; public RECT caretRect; }
  [StructLayout(LayoutKind.Sequential)] public struct MSG { public IntPtr hwnd; public uint message; public IntPtr w, l; public uint time; public POINT pt; public uint priv; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] public struct IU { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public IU u; }

  public static System.Diagnostics.Stopwatch Clock = System.Diagnostics.Stopwatch.StartNew();
  public static List<string> Events = new List<string>();
  public static Dictionary<long, int> Ids = new Dictionary<long, int>();
  public static Dictionary<uint, int> Tids = new Dictionary<uint, int>();
  public static string Step = "start";
  public static string Id(IntPtr h) {
    if (h == IntPtr.Zero) return "-";
    int n; if (!Ids.TryGetValue(h.ToInt64(), out n)) { n = Ids.Count + 1; Ids[h.ToInt64()] = n; }
    return "w" + n;
  }
  public static string Tid(uint t) {
    int n; if (!Tids.TryGetValue(t, out n)) { n = Tids.Count + 1; Tids[t] = n; }
    return "t" + n;
  }
  public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassNameW(h, s, 256); return s.ToString(); }
  public static string Text(IntPtr h) { var s = new StringBuilder(512); GetWindowTextW(h, s, 512); return s.ToString(); }
  public static uint Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
  // Poll instead of a WinEvent hook: an out-of-context hook made chrome.exe crash (breakpoint in chrome.dll,
  // 0x80000003) before its first window was shown, so the order of events is seen by polling every ~10 ms.
  public static Dictionary<long, string> Seen = new Dictionary<long, string>();
  static long lastFg;
  public static HashSet<uint> Pids = new HashSet<uint>();
  public static void StartHook() { }
  public static void StopHook() { }
  public static void Poll() {
    var now = new HashSet<long>();
    EnumWindows((h, p) => { PollOne(h, now); EnumChildWindows(h, (c, q) => { PollOne(c, now); return true; }, IntPtr.Zero); return true; }, IntPtr.Zero);
    var gone = new List<long>();
    foreach (var k in Seen.Keys) if (!now.Contains(k)) gone.Add(k);
    foreach (var k in gone) { Events.Add(string.Format("{0,6} {1} destroyed {2}", Clock.ElapsedMilliseconds, Step, Seen[k].Split('|')[0])); Seen.Remove(k); }
    long fg = GetForegroundWindow().ToInt64();
    if (fg != lastFg) { Events.Add(string.Format("{0,6} {1} foreground {2} {3}", Clock.ElapsedMilliseconds, Step, Id(new IntPtr(fg)), fg == 0 ? "" : Class(new IntPtr(fg)))); lastFg = fg; }
  }
  static void PollOne(IntPtr h, HashSet<long> now) {
    uint pid = Pid(h);
    string cls = Class(h);
    if (!IsChrome(pid, cls)) return;
    now.Add(h.ToInt64());
    RECT wr; GetWindowRect(h, out wr);
    long st = GetWindowLongPtr(h, -16).ToInt64() & 0xFFFFFFFFL, ex = GetWindowLongPtr(h, -20).ToInt64() & 0xFFFFFFFFL;
    string head = Id(h) + " " + cls + " " + (Pids.Contains(pid) ? "browser" : "other");
    string now_ = head + "|parent=" + Id(GetParent(h)) + " owner=" + Id(GetWindow(h, 4)) + " style=" + st.ToString("X8") + " ex=" + ex.ToString("X8") + " vis=" + (IsWindowVisible(h) ? 1 : 0) + " rect=" + R(wr) + " text=\"" + Text(h) + "\"";
    string old;
    if (!Seen.TryGetValue(h.ToInt64(), out old)) { Events.Add(string.Format("{0,6} {1} created   {2}", Clock.ElapsedMilliseconds, Step, now_.Replace("|", " "))); }
    else if (old != now_) { Events.Add(string.Format("{0,6} {1} changed   {2} -> {3}", Clock.ElapsedMilliseconds, Step, head, Diff(old.Split('|')[1], now_.Split('|')[1]))); }
    Seen[h.ToInt64()] = now_;
  }
  static string Diff(string a, string b) {
    var pa = a.Split(' '); var pb = b.Split(' '); var sb = new StringBuilder();
    // text may contain spaces; compare only the leading key=value fields
    for (int i = 0; i < pa.Length && i < pb.Length; i++) { if (pa[i] != pb[i] && pb[i].Contains("=")) sb.Append(pa[i] + " => " + pb[i] + "; "); }
    if (a.Substring(a.IndexOf("text=")) != b.Substring(b.IndexOf("text="))) sb.Append("text " + b.Substring(b.IndexOf("text=")));
    return sb.ToString();
  }
  public static void Pump(int ms) {
    var t = System.Diagnostics.Stopwatch.StartNew();
    while (t.ElapsedMilliseconds < ms) {
      MSG m; while (PeekMessageW(out m, IntPtr.Zero, 0, 0, 1)) DispatchMessageW(ref m);
      Poll();
      System.Threading.Thread.Sleep(8);
    }
  }
  public static List<IntPtr> All(HashSet<uint> pids) {
    var l = new List<IntPtr>();
    EnumWindows((h, p) => {
      if (pids.Contains(Pid(h))) l.Add(h);
      EnumChildWindows(h, (c, q) => { if (pids.Contains(Pid(c))) l.Add(c); return true; }, IntPtr.Zero);
      return true; }, IntPtr.Zero);
    return l;
  }
  static string R(RECT r) { return r.L + "," + r.T + "," + r.R + "," + r.B; }
  public static string Describe(IntPtr h, string role) {
    var sb = new StringBuilder();
    RECT wr, cr, rb; GetWindowRect(h, out wr); GetClientRect(h, out cr);
    uint pid; uint tid = GetWindowThreadProcessId(h, out pid);
    long st = GetWindowLongPtr(h, -16).ToInt64() & 0xFFFFFFFFL, ex = GetWindowLongPtr(h, -20).ToInt64() & 0xFFFFFFFFL;
    int rt = GetWindowRgnBox(h, out rb);
    uint key, fl; byte al; bool lay = GetLayeredWindowAttributes(h, out key, out al, out fl);
    uint aff; bool haff = GetWindowDisplayAffinity(h, out aff);
    int ncr = -1, cloak = -1, hr1 = DwmGetWindowAttribute(h, 1, out ncr, 4), hr2 = DwmGetWindowAttribute(h, 14, out cloak, 4);
    RECT fb; int hr3 = DwmGetWindowAttribute(h, 9, out fb, 16);
    sb.AppendFormat("{0} {1} role={2} thread={3} parent={4} owner={5} root={6} style={7:X8} ex={8:X8} cls_style={9:X4} id={10} vis={11} icon={12} zoom={13}",
      Id(h), Class(h), role, Tid(tid), Id(GetParent(h)), Id(GetWindow(h, 4)), Id(GetAncestor(h, 2)), st, ex,
      GetClassLongPtr(h, -26).ToInt64() & 0xFFFF, GetWindowLongPtr(h, -12).ToInt64(), IsWindowVisible(h) ? 1 : 0, IsIconic(h) ? 1 : 0, IsZoomed(h) ? 1 : 0);
    var wp = new WINDOWPLACEMENT(); wp.length = (uint)Marshal.SizeOf(typeof(WINDOWPLACEMENT)); bool okp = GetWindowPlacement(h, ref wp);
    long bg = GetClassLongPtr(h, -10).ToInt64();
    sb.AppendFormat(" class(bg={0} cursor={1} icon={2} iconsm={3} wndextra={4} clsextra={5}) place={6}", bg == 0 ? "null" : bg < 64 ? "sys" + (bg - 1) : "brush",
      GetClassLongPtr(h, -12) == IntPtr.Zero ? "null" : "set", GetClassLongPtr(h, -14) == IntPtr.Zero ? "null" : "set", GetClassLongPtr(h, -34) == IntPtr.Zero ? "null" : "set",
      GetClassLongPtr(h, -18).ToInt64(), GetClassLongPtr(h, -20).ToInt64(),
      okp ? "flags=" + wp.flags + ",show=" + wp.showCmd + ",min=" + wp.ptMin.x + "/" + wp.ptMin.y + ",max=" + wp.ptMax.x + "/" + wp.ptMax.y + ",normal=" + R(wp.normal) : "fail");
    sb.AppendFormat(" rect={0} client={1} rgn={2}{3} layered={4}{5} affinity={6} ncr={7}/{8} cloak={9}/{10} frame={11}/{12} text=\"{13}\"",
      R(wr), R(cr), rt, rt > 0 ? "[" + R(rb) + "]" : "", lay ? 1 : 0, lay ? "[a=" + al + ",f=" + fl + "]" : "", haff ? aff.ToString() : "n", hr1, ncr, hr2, cloak, hr3, hr3 == 0 ? R(fb) : "-", Text(h));
    return sb.ToString();
  }
  public static void Click(int x, int y) {
    SetCursorPos(x, y);
    var i = new INPUT[2];
    i[0].type = 0; i[0].u.mi.flags = 2;
    i[1].type = 0; i[1].u.mi.flags = 4;
    SendInput(2, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void TypeChar(char c) {
    var i = new INPUT[2];
    i[0].type = 1; i[0].u.ki.scan = c; i[0].u.ki.flags = 4;
    i[1].type = 1; i[1].u.ki.scan = c; i[1].u.ki.flags = 4 | 2;
    SendInput(2, i, Marshal.SizeOf(typeof(INPUT)));
  }
}
'@

New-Item -ItemType Directory -Force $Out | Out-Null
if (Test-Path $Ud) { Remove-Item $Ud -Recurse -Force }
$log = New-Object System.Collections.Generic.List[string]
function L([string]$s) { $log.Add($s) }
if (-not $NoHook) { [O]::StartHook() }
$args2 = @('--disable-gpu', '--no-first-run', "--user-data-dir=$Ud", '--window-position=40,40', '--window-size=900,500', $Page)
if ($Gpu) { $args2 = @($args2 | Where-Object { $_ -ne '--disable-gpu' }) }
$proc = Start-Process -FilePath "$ChromeDir\chrome.exe" -ArgumentList $args2 -PassThru
$null = $proc.Handle
[void][O]::Pids.Add([uint32]$proc.Id)
$browserPid = [uint32]$proc.Id

function ChromePids {
  $s = New-Object 'System.Collections.Generic.HashSet[uint32]'
  foreach ($p in Get-Process -Name chrome -ErrorAction SilentlyContinue) { [void]$s.Add([uint32]$p.Id) }
  return , $s
}
function Role([uint32]$procId) {
  if ($procId -eq $browserPid) { return 'browser' }
  $c = Get-CimInstance Win32_Process -Filter "ProcessId=$procId" -ErrorAction SilentlyContinue
  if ($c -and $c.CommandLine -match '--type=([\w-]+)') { return $Matches[1] }
  return 'other'
}
function Dump([string]$title) {
  $pids = ChromePids
  $roles = @{}
  foreach ($p in $pids) { $roles[$p] = Role $p }
  L "--- $title (t=$([O]::Clock.ElapsedMilliseconds) ms, chrome processes: $(($roles.GetEnumerator() | Group-Object Value | ForEach-Object { "$($_.Name)=$($_.Count)" }) -join ' '))"
  $fg = [O]::GetForegroundWindow()
  L "foreground: $([O]::Id($fg)) $([O]::Class($fg))"
  $mainw = Find-Main
  if ($mainw -ne [IntPtr]::Zero) {
    $gi = New-Object O+GUITHREADINFO; $gi.size = [Runtime.InteropServices.Marshal]::SizeOf([type][O+GUITHREADINFO])
    $pv = [uint32]0; $tid = [O]::GetWindowThreadProcessId($mainw, [ref]$pv)
    if ([O]::GetGUIThreadInfo($tid, [ref]$gi)) { L "ui thread: active=$([O]::Id($gi.active)) focus=$([O]::Id($gi.focus)) capture=$([O]::Id($gi.capture)) flags=$($gi.flags)" }
  }
  $wins = [O]::All($pids)
  foreach ($h in $wins) { L ([O]::Describe($h, $roles[[O]::Pid($h)])) }
}
function Find-Main {
  foreach ($h in [O]::All((ChromePids))) {
    if ([O]::Class($h) -eq 'Chrome_WidgetWin_1' -and [O]::IsWindowVisible($h) -and [O]::Text($h) -like 'poly-s3*') { return $h }
  }
  return [IntPtr]::Zero
}

# a: wait for the main window
[O]::Step = 'a'
$h = [IntPtr]::Zero
$t0 = [Diagnostics.Stopwatch]::StartNew()
while ($t0.Elapsed.TotalSeconds -lt 90) { [O]::Pump(250); $h = Find-Main; if ($h -ne [IntPtr]::Zero) { break } }
if ($h -eq [IntPtr]::Zero) { L 'no window'; L "exited=$($proc.HasExited) code=$(if ($proc.HasExited) { $proc.ExitCode })"; Dump 'no window'; foreach ($e in [O]::Events) { L $e }; $log | Set-Content -Encoding UTF8 "$Out\s3win.txt"; try { $proc.Kill() } catch {}; 'done' | Set-Content "$Out\done.txt"; exit 1 }
[O]::Pump(3000)
Dump 'a: first visible'

# b: place, click, type
[O]::Step = 'b'
[void][O]::SetWindowPos($h, [IntPtr]::Zero, 40, 40, 900, 500, 0x44)
[void][O]::SetForegroundWindow($h)
[O]::Pump(800)
$r = New-Object O+RECT; [void][O]::GetWindowRect($h, [ref]$r)
$cx = [int](($r.L + $r.R) / 2); $cy = [int]($r.T + ($r.B - $r.T) * 0.7)
[void][O]::SetCursorPos($cx - 100, $cy); [O]::Pump(300)
[void][O]::SetCursorPos($cx - 50, $cy + 10); [O]::Pump(300)
[O]::Click($cx, $cy); [O]::Pump(600)
foreach ($c in 'p', 'o', 'l', 'y') { [O]::TypeChar([char]$c); [O]::Pump(150) }
[O]::Pump(1500)
Dump "b: after click and keys (title: $([O]::Text($h)))"

# c: resize
[O]::Step = 'c'
[void][O]::SetWindowPos($h, [IntPtr]::Zero, 60, 60, 700, 400, 0x44)
[O]::Pump(1500)
Dump 'c: after SetWindowPos 60,60 700x400'

# d: minimize, restore
[O]::Step = 'd'
[void][O]::ShowWindow($h, 6); [O]::Pump(1500)
Dump 'd1: minimized'
[void][O]::ShowWindow($h, 9); [O]::Pump(1500)
Dump 'd2: restored'

# e: close
[O]::Step = 'e'
[void][O]::PostMessageW($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
$t1 = [Diagnostics.Stopwatch]::StartNew()
while ($t1.Elapsed.TotalSeconds -lt 20 -and -not $proc.HasExited) { [O]::Pump(200) }
[O]::Pump(1000)
L "--- e: browser exited=$($proc.HasExited) exit=$(if ($proc.HasExited) { $proc.ExitCode } else { 'n/a' }) at t=$([O]::Clock.ElapsedMilliseconds) ms"
if (-not $NoHook) { [O]::StopHook() }

L '--- events (polled every ~10 ms from outside; times in ms from the start of the run)'
foreach ($e in [O]::Events) { L $e }
$log | Set-Content -Encoding UTF8 "$Out\s3win.txt"
'done' | Set-Content "$Out\done.txt"
