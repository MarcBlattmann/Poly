param(
  [Parameter(Mandatory)][string]$Stage,   # s1 s2 s4 s5
  [Parameter(Mandatory)][string]$Run,
  [string]$Url,
  [int]$TimeoutSec = 180,
  [string]$Work = "$env:USERPROFILE\pc",
  [string]$ChromeDir
)
$ErrorActionPreference = 'Stop'
$P = $Work
if (-not $ChromeDir) { $ChromeDir = "$Work\chrome" }
if (-not $Url) { $Url = 'file:///' + ($Work -replace '\\', '/') + '/pages/s1.html' }
$out = "$P\tr\$Stage-run$Run"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null
$ud = "$P\ud\$Stage-$Run"
if (Test-Path $ud) { Remove-Item $ud -Recurse -Force }
$flags = switch ($Stage) {
  's1' { @('--headless', '--no-sandbox', '--disable-gpu') }
  's2' { @('--headless', '--disable-gpu') }
  's4' { @('--headless', '--no-sandbox', '--disable-gpu') }
  's5' { @('--headless', '--no-sandbox') }
  default { throw "unknown stage $Stage" }
}
$env:POLYTRACE_OUT = "$out\polytrace-%p.jsonl"
$args2 = $flags + @("--user-data-dir=$ud", '--dump-dom', $Url)
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath "$ChromeDir\chrome.exe" -ArgumentList $args2 -RedirectStandardOutput "$out\stdout.txt" -RedirectStandardError "$out\stderr.txt" -PassThru -NoNewWindow; $null = $p.Handle
if (-not $p.WaitForExit($TimeoutSec * 1000)) { $p.Kill(); "TIMEOUT" }
Remove-Item Env:POLYTRACE_OUT
Start-Sleep -Seconds 2
"exit=$($p.ExitCode) seconds=$([math]::Round($sw.Elapsed.TotalSeconds,1))"
Get-ChildItem $out | Select Name, Length | Format-Table -AutoSize | Out-String
Get-Content "$out\stdout.txt" -TotalCount 8

