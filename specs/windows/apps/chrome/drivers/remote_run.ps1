param([string]$Stage='s3',[string]$Kind='orig',[string]$Run='1',[switch]$Traced,[switch]$Gpu)
$ErrorActionPreference='Stop'
$W="$HOME\pc"
$out="$W\tr\$Stage-$Kind-run$Run"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null
$a="-ChromeDir $W\$Kind -Out $out -Work $W -Stage $Stage"
if ($Traced) { $a += " -TraceDir $out -NoSandbox" }
if ($Gpu) { $a += " -Gpu" }
& "$W\scripts\interactive_task.ps1" -Script "$W\scripts\s3_driver.ps1" -ArgLine $a -DoneFile "$out\result.json" -TimeoutSec 240
