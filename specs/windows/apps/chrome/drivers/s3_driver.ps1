# Stages S3 and S4 of the Chrome scenario (specs/windows/apps/chrome): a visible Chrome window.
#   S3: opens a local page, renders text, takes mouse and keyboard input, closes cleanly.
#   S4: S3 with the network: loads http://example.com/, then (address bar) https://www.iana.org/.
# Observes only through Win32 (window titles, a screenshot, the exit code); needs the interactive
# desktop (a locked session or a session 0 service cannot do it: SendInput, CopyFromScreen).
param(
  [Parameter(Mandatory)][string]$ChromeDir,
  [Parameter(Mandatory)][string]$Out,        # result.json and the screenshot go here
  [string]$TraceDir,                         # set => POLYTRACE_OUT=<TraceDir>\polytrace-%p.jsonl (traced kit)
  [string]$Work = "$env:USERPROFILE\pc",     # the work directory: pages\ and ud\ (profiles) are below it
  [string]$Page,                             # default: <Work>\pages\s3.html
  [string]$Ud,                               # default: <Work>\ud\<stage>
  [ValidateSet('s3', 's4')][string]$Stage = 's3',
  [switch]$Gpu,                              # S5: leave the GPU flags at their defaults (no --disable-gpu)
  [switch]$NoSandbox                         # the traced kit cannot run sandboxed renderers (see the gap map)
)
$ErrorActionPreference = 'Stop'
if (-not $Page) { $Page = 'file:///' + ($Work -replace '\\', '/') + '/pages/s3.html' }
if (-not $Ud) { $Ud = "$Work\ud\$Stage" }
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] i, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] public struct IU { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public IU u; }
  public static List<IntPtr> TopWindows() {
    var l = new List<IntPtr>();
    EnumWindows((h, p) => { l.Add(h); return true; }, IntPtr.Zero);
    return l;
  }
  public static string Text(IntPtr h) { var s = new StringBuilder(512); GetWindowTextW(h, s, 512); return s.ToString(); }
  public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassNameW(h, s, 256); return s.ToString(); }
  public static uint Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
  public static void Click(int x, int y) {
    SetCursorPos(x, y);
    var i = new INPUT[2];
    i[0].type = 0; i[0].u.mi.flags = 2;  // MOUSEEVENTF_LEFTDOWN
    i[1].type = 0; i[1].u.mi.flags = 4;  // MOUSEEVENTF_LEFTUP
    SendInput(2, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void Key(ushort vk, bool up) {
    var i = new INPUT[1];
    i[0].type = 1; i[0].u.ki.vk = vk; i[0].u.ki.flags = up ? 2u : 0u;
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void TypeChar(char c) {
    var i = new INPUT[2];
    i[0].type = 1; i[0].u.ki.scan = c; i[0].u.ki.flags = 4;        // KEYEVENTF_UNICODE
    i[1].type = 1; i[1].u.ki.scan = c; i[1].u.ki.flags = 4 | 2;    // ... | KEYEVENTF_KEYUP
    SendInput(2, i, Marshal.SizeOf(typeof(INPUT)));
  }
}
'@

New-Item -ItemType Directory -Force $Out | Out-Null
if (Test-Path $Ud) { Remove-Item $Ud -Recurse -Force }
$result = [ordered]@{ stage = $Stage }
$sw = [Diagnostics.Stopwatch]::StartNew()
if ($TraceDir) { New-Item -ItemType Directory -Force $TraceDir | Out-Null; $env:POLYTRACE_OUT = "$TraceDir\polytrace-%p.jsonl" }
$startUrl = if ($Stage -eq 's4') { 'http://example.com/' } else { $Page }
$titlePrefix = if ($Stage -eq 's4') { 'Example Domain*' } else { 'poly-s3*' }
$args2 = @('--disable-gpu', '--no-first-run', "--user-data-dir=$Ud", '--window-position=40,40', '--window-size=900,500', $startUrl)
if ($NoSandbox) { $args2 = @('--no-sandbox') + $args2 }
if ($Gpu) { $args2 = @($args2 | Where-Object { $_ -ne '--disable-gpu' }) }
$proc = Start-Process -FilePath "$ChromeDir\chrome.exe" -ArgumentList $args2 -PassThru
$null = $proc.Handle
if ($TraceDir) { Remove-Item Env:POLYTRACE_OUT }

function Find-Main([string]$like) {
  foreach ($h in [W]::TopWindows()) {
    if ([W]::Class($h) -eq 'Chrome_WidgetWin_1' -and [W]::IsWindowVisible($h) -and [W]::Text($h) -like $like) { return $h }
  }
  return [IntPtr]::Zero
}
function Wait-Title([string]$like, [int]$seconds) {
  $t0 = [Diagnostics.Stopwatch]::StartNew()
  while ($t0.Elapsed.TotalSeconds -lt $seconds) {
    $h = Find-Main $like
    if ($h -ne [IntPtr]::Zero) { return $h }
    Start-Sleep -Milliseconds 250
  }
  return [IntPtr]::Zero
}
function Type-Text([string]$s) { foreach ($c in $s.ToCharArray()) { [W]::TypeChar($c); Start-Sleep -Milliseconds 40 } }

# 1. a window with the page's title
$h = Wait-Title $titlePrefix 90
if ($h -eq [IntPtr]::Zero) { $result.error = 'no window'; $result.exited = $proc.HasExited; $result | ConvertTo-Json | Set-Content -Encoding UTF8 "$Out\result.json"; try { $proc.Kill() } catch {}; exit 1 }
$result.window_seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1)
$result.title_initial = [W]::Text($h)
$result.window_class = [W]::Class($h)
Start-Sleep -Seconds 3

# 2. place and bring to the front
[void][W]::SetWindowPos($h, [IntPtr]::Zero, 40, 40, 900, 500, 0x44)   # SWP_NOZORDER | SWP_SHOWWINDOW
[void][W]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 800
$r = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$r)
$result.rect = "$($r.L),$($r.T),$($r.R),$($r.B)"
$cx = [int](($r.L + $r.R) / 2); $cy = [int]($r.T + ($r.B - $r.T) * 0.7)

if ($Stage -eq 's3') {
  # 3a. mouse move + click in the page, then keys
  [void][W]::SetCursorPos($cx - 100, $cy); Start-Sleep -Milliseconds 300
  [void][W]::SetCursorPos($cx - 50, $cy + 10); Start-Sleep -Milliseconds 300
  [W]::Click($cx, $cy); Start-Sleep -Milliseconds 600
  foreach ($c in 'p', 'o', 'l', 'y') { [W]::TypeChar([char]$c); Start-Sleep -Milliseconds 150 }
  $want = 'poly-s3 clicks=1 keys=poly moved=1'
  $t = ''
  for ($i = 0; $i -lt 40; $i++) { $t = [W]::Text($h); if ($t -like "$want*") { break }; Start-Sleep -Milliseconds 250 }
  $result.title_after_input = $t
  $result.input_ok = ($t -like "$want*")
} else {
  # 3b. the first page is the http one; go to the https one through the address bar (Ctrl+L)
  $result.title_http = [W]::Text($h)
  $result.http_ok = ($result.title_http -like 'Example Domain*')
  [W]::Click($cx, $cy); Start-Sleep -Milliseconds 400
  [W]::Key(0xA2, $false); [W]::Key(0x4C, $false); [W]::Key(0x4C, $true); [W]::Key(0xA2, $true)   # Ctrl+L
  Start-Sleep -Milliseconds 500
  Type-Text 'https://www.iana.org/'
  [W]::Key(0x0D, $false); [W]::Key(0x0D, $true)                                                    # Enter
  $h2 = Wait-Title 'Internet Assigned Numbers Authority*' 60
  if ($h2 -ne [IntPtr]::Zero) { $h = $h2 }
  $result.title_https = [W]::Text($h)
  $result.https_ok = ($result.title_https -like 'Internet Assigned Numbers Authority*')
  $result.input_ok = ($result.http_ok -and $result.https_ok)
  Start-Sleep -Seconds 2
}

# 4. a screenshot of the window (is text drawn?)
[void][W]::GetWindowRect($h, [ref]$r)
$w = $r.R - $r.L; $hh = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size $w, $hh))
$g.Dispose()
$shot = "$Out\$Stage-window.png"
$bmp.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
$dark = 0; $total = 0
for ($y = [int]($hh * 0.25); $y -lt [int]($hh * 0.9); $y += 2) { for ($x = 20; $x -lt [int]($w * 0.8); $x += 2) { $px = $bmp.GetPixel($x, $y); $total++; if (($px.R + $px.G + $px.B) -lt 240) { $dark++ } } }
$bmp.Dispose()
$result.screenshot = "$Stage-window.png"
$result.dark_pixels_sampled = $dark
$result.pixels_sampled = $total
if ($Stage -eq 's3') { $result.text_rendered = ($dark -gt 150) }

# 5. close cleanly (WM_CLOSE to the main window)
[void][W]::PostMessageW($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
$exited = $proc.WaitForExit(30000)
if (-not $exited) { $result.close = 'timeout'; try { $proc.Kill() } catch {} } else { $result.close = 'exited'; $result.exit_code = $proc.ExitCode }
Start-Sleep -Seconds 3
$result.total_seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1)
$result | ConvertTo-Json | Set-Content -Encoding UTF8 "$Out\result.json"
$result | ConvertTo-Json
