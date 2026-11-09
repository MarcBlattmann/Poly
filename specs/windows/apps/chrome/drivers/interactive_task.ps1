# Run a script in the interactive desktop session of the current user through a one-time scheduled
# task (the way tools/pipeline/windows/poly-oracle.ps1 -Interactive does, ADR 0075/0097) and wait
# for it to write <DoneFile>. Refuses when the user has no interactive session or it is locked.
param(
  [Parameter(Mandatory)][string]$Script,
  [Parameter(Mandatory)][string]$ArgLine,     # arguments for the script, as one command-line string
  [Parameter(Mandatory)][string]$DoneFile,
  [int]$TimeoutSec = 400
)
$ErrorActionPreference = 'Stop'
$user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$mine = @(Get-CimInstance Win32_Process -Filter "Name='explorer.exe'" | Where-Object {
    $_.SessionId -ne 0 -and (Invoke-CimMethod -InputObject $_ -MethodName GetOwner).User -eq $env:USERNAME })
if ($mine.Count -eq 0) { throw "$user is not logged on interactively (no explorer.exe of theirs)" }
$session = $mine[0].SessionId
if (@(Get-Process LogonUI -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session }).Count -gt 0) {
  throw "session $session of $user is locked"
}
if (Test-Path $DoneFile) { Remove-Item $DoneFile -Force }
$name = 'poly-chrome-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$Script`" $ArgLine" -WorkingDirectory (Split-Path $Script)
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Seconds ($TimeoutSec + 60)) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $name -Action $action -Principal $principal -Settings $settings | Out-Null
try {
  "running in session $session of $user (task $name)"
  Start-ScheduledTask -TaskName $name
  $watch = [Diagnostics.Stopwatch]::StartNew()
  while (-not (Test-Path $DoneFile)) {
    Start-Sleep -Milliseconds 700
    if ($watch.Elapsed.TotalSeconds -gt $TimeoutSec) { Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue; "TIMEOUT"; break }
    $state = [string](Get-ScheduledTask -TaskName $name).State
    if ($state -eq 'Ready' -and $watch.Elapsed.TotalSeconds -gt 5 -and -not (Test-Path $DoneFile)) { Start-Sleep -Seconds 2; if (-not (Test-Path $DoneFile)) { "task ended without $DoneFile (result $((Get-ScheduledTaskInfo -TaskName $name).LastTaskResult))"; break } }
  }
} finally {
  Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction SilentlyContinue
}
if (Test-Path $DoneFile) { Get-Content $DoneFile -Raw }
