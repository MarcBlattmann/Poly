# Which delay-loaded DLLs can Chrome (S1: headless --no-sandbox --disable-gpu --dump-dom) do without?
# For each proxy of a delay-loaded DLL: move pt_<dll>.dll away (the delay-load of that DLL then fails), run S1,
# record exit code and whether stdout is the expected DOM, put the file back.
param([string]$Work = "$env:USERPROFILE\pc", [int]$TimeoutSec = 40)
$ErrorActionPreference = 'Stop'
$k = Get-Content "$Work\kit\polytrace-kit.json" -Raw | ConvertFrom-Json
$delay = (Get-Content "$Work\delay-dlls.txt")
$expect = '7b79f8b475542cce0f029ef2459918c36404fe1986144f2fd1f2bf8aea13807b'
$res = @()
foreach ($d in $delay) {
  $f = "$Work\chrome\pt_$d"
  if (-not (Test-Path $f)) { continue }
  Move-Item $f "$f.off"
  $out = "$Work\tr\ablate"; if (Test-Path $out) { Get-ChildItem $out | % { Remove-Item $_.FullName -Recurse -Force } } else { New-Item -ItemType Directory $out | Out-Null }
  $env:POLYTRACE_OUT = "$out\polytrace-%p.jsonl"; $ud = "$Work\ud\ablate"; if (Test-Path $ud) { Remove-Item $ud -Recurse -Force }
  $p = Start-Process "$Work\chrome\chrome.exe" -ArgumentList @('--headless', '--no-sandbox', '--disable-gpu', "--user-data-dir=$ud", '--dump-dom', ('file:///' + ($Work -replace '\\', '/') + '/pages/s1.html')) -RedirectStandardOutput "$out\stdout.txt" -RedirectStandardError "$out\stderr.txt" -PassThru -NoNewWindow
  $null = $p.Handle
  $to = $false
  if (-not $p.WaitForExit($TimeoutSec * 1000)) { $p.Kill(); $to = $true }
  Start-Sleep -Milliseconds 800
  Get-Process chrome -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Work\*" } | Stop-Process -Force -ErrorAction SilentlyContinue
  Move-Item "$f.off" $f
  $sha = ''
  try { $sha = (Get-FileHash "$out\stdout.txt").Hash.ToLower() } catch {}
  $row = [ordered]@{ dll = $d; exit = $p.ExitCode; timeout = $to; dom_ok = ($sha -eq $expect) }
  $res += [pscustomobject]$row
  "{0,-40} exit={1} timeout={2} dom_ok={3}" -f $d, $p.ExitCode, $to, ($sha -eq $expect)
  Start-Sleep -Seconds 1
}
$res | ConvertTo-Json | Set-Content "$Work\tr\ablate.json"

