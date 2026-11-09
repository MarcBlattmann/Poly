param(
  [Parameter(Mandatory)][string]$Run,       # run label, e.g. 1
  [string]$Work = "$env:USERPROFILE\pc3",
  [string]$AppDir,                          # the application with the kit overlaid (default: $Work\appk)
  [int]$TimeoutSec = 120,
  [string]$Tag = 'cs',
  [switch]$Cold                             # run with the two HKCU keys Chrome for Testing keeps absent (restored afterwards)
)
# S1 (headless, --no-sandbox, --disable-gpu, --dump-dom) of the Chrome-for-Testing kit built from
# chrome_elf.dll and chrome.exe only (tools/wintrace/polytrace-sync): one run, browser trace in
# $Work\tr\$Tag-run$Run\polytrace.jsonl. It starts one process tree and waits for it; it never
# kills anything but the process it started.
$ErrorActionPreference = 'Stop'
if (-not $AppDir) { $AppDir = "$Work\appk" }
$out = "$Work\tr\$Tag-run$Run"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null
$ud = "$Work\ud\$Tag-$Run"
if (Test-Path $ud) { Remove-Item $ud -Recurse -Force }
# The parent of the profile directory exists before the run: Chrome's RecursiveDirectoryCreate would
# otherwise make it in the first run only (two more calls: GetFileAttributesW, CreateDirectoryW).
New-Item -ItemType Directory -Force "$Work\ud" | Out-Null
$page = 'file:///' + ($Work -replace '\\', '/') + '/pages/s1.html'
# Cold: Chrome for Testing's own bookkeeping keys (the blocklist beacon, the ThirdParty status codes, the
# usage-stats setting) do not exist, as on a machine that never ran it (Poly's registry at start). They are
# saved first and put back afterwards.
$coldKeys = 'HKCU\Software\Chrome for Testing', 'HKCU\Software\Google\Chrome for Testing'
if ($Cold) {
  $i = 0
  foreach ($k in $coldKeys) {
    $i++
    cmd.exe /c "reg.exe export `"$k`" `"$out\backup-$i.reg`" /y >nul 2>&1"
    cmd.exe /c "reg.exe delete `"$k`" /f >nul 2>&1"
  }
}
try {
  $env:POLYTRACE_SYNC = '1'
  $env:POLYTRACE_OUT = "$out\polytrace.jsonl"
  if ($env:POLYTRACE_DUMP -eq $null) { $env:POLYTRACE_DUMP = '1' }
  $args2 = @('--headless', '--no-sandbox', '--disable-gpu', "--user-data-dir=$ud", '--dump-dom', $page)
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath "$AppDir\chrome.exe" -ArgumentList $args2 -WorkingDirectory $AppDir `
    -RedirectStandardOutput "$out\stdout.txt" -RedirectStandardError "$out\stderr.txt" -PassThru -NoNewWindow
  $null = $p.Handle
  if (-not $p.WaitForExit($TimeoutSec * 1000)) { $p.Kill(); 'TIMEOUT' }
} finally {
  Remove-Item Env:POLYTRACE_OUT, Env:POLYTRACE_SYNC -ErrorAction SilentlyContinue
  if ($Cold) {
    $i = 0
    foreach ($k in $coldKeys) {
      $i++
      cmd.exe /c "reg.exe delete `"$k`" /f >nul 2>&1"
      if (Test-Path "$out\backup-$i.reg") { cmd.exe /c "reg.exe import `"$out\backup-$i.reg`" >nul 2>&1" }
    }
  }
}
Start-Sleep -Seconds 2
"exit=$($p.ExitCode) seconds=$([math]::Round($sw.Elapsed.TotalSeconds,1)) pid=$($p.Id)"
Get-ChildItem $out | Select-Object Name, Length | Format-Table -AutoSize | Out-String
(Get-Content "$out\stdout.txt" -Raw).Length
