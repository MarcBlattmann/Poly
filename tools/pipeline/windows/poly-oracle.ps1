<#
.SYNOPSIS
Poly's Windows test oracle runner (docs/adr/0075-windows-test-oracle.md).

.DESCRIPTION
Runs Poly's Windows test programs on this (real) Windows machine and writes
what they did into one JSON result file (format poly-winrun/1), which the
pipeline on the Linux side compares with Poly:
`cargo xtask pipeline win diff --reference results.json`.

The test programs are built on the Linux side (`cargo xtask pipeline win
build`, `win bundle`) and arrive here as a bundle: one text file
(poly-winbundle/1), or several parts of it given in order. Every file in it
carries its SHA-256, which is checked before anything runs.

Each test runs -Runs times (default 2), each time as a copy of itself in a
fresh scratch directory below -Work, which is also its current directory,
with standard input at end of file and standard output and error captured.
A test writes its observations into `poly-test.out` beside itself; that
file, the exit code (32 bits), stdout and stderr are recorded. A test can
have data files (ADR 0088): the files of <DataDir>\<test>\ (or the bundle's
`data` lines for it) are copied beside it into every scratch directory
before it runs, so a test such as the 7-Zip scenario driver finds the
program it runs there. A run that
takes longer than -TimeoutSeconds is killed. Runs that differ make the test
"stable": false (a test must be deterministic).

Windows PowerShell 5.1 or later; nothing to install, nothing changed outside
-Work and -Out. Run it with a process-scoped execution policy:

  powershell -NoProfile -ExecutionPolicy Bypass -File poly-oracle.ps1 -Bundle bundle.txt -Out results.json

.PARAMETER Bundle
The bundle file, or its parts in order (bundle.txt.part1of3, ...).

.PARAMETER TestsDir
Instead of a bundle: a directory of test .exe files.

.PARAMETER DataDir
A directory with one subdirectory per test that has data files
(<DataDir>\sevenzip_scenario\7za.exe); each subdirectory's files are copied
beside that test before each run. A bundle's `data` lines are unpacked into
<Work>\data, which is the default when the bundle has any.

.PARAMETER TestTimeouts
Per-test timeouts in seconds, overriding -TimeoutSeconds:
'sevenzip_scenario=300' (several separated by commas).

.PARAMETER Out
The result file to write (default: results.json in the current directory).

.PARAMETER Tests
Only these tests (names without .exe).

.PARAMETER Work
Where executables and scratch directories go (default: %TEMP%\poly-oracle).
It is emptied first.

.PARAMETER PrintJson
Also print the result file on standard output (to copy it from a session).
#>
[CmdletBinding()]
param(
    [string[]] $Bundle,
    [string] $TestsDir,
    [string] $DataDir,
    [string[]] $TestTimeouts,
    [string] $Out = 'results.json',
    [string[]] $Tests,
    [int] $TimeoutSeconds = 10,
    [int] $Runs = 2,
    [string] $Work = (Join-Path ([IO.Path]::GetTempPath()) 'poly-oracle'),
    [switch] $KeepScratch,
    [switch] $PrintJson
)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'

$RunnerVersion = 'poly-oracle.ps1 2'
$RunFormat = 'poly-winrun/1'
$BundleFormat = 'poly-winbundle/1'
$ObservationFile = 'poly-test.out'

function Resolve-UserPath([string] $Path) {
    # [IO.File] resolves against the process's directory, not PowerShell's.
    return $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)
}

function Get-Sha256Hex([byte[]] $Bytes) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $hash = $sha.ComputeHash($Bytes)
    } finally {
        $sha.Dispose()
    }
    return (($hash | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Get-Prop($Object, [string] $Name) {
    $p = $Object.PSObject.Properties[$Name]
    if ($null -eq $p) { return $null }
    return $p.Value
}

function Get-SystemInfo {
    $cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $build = [string](Get-Prop $cv 'CurrentBuild')
    $ubr = Get-Prop $cv 'UBR'
    if ($null -ne $ubr) { $build = "$build.$ubr" }
    return [ordered]@{
        name    = 'windows'
        version = [Environment]::OSVersion.Version.ToString()
        build   = $build
        arch    = $env:PROCESSOR_ARCHITECTURE
        edition = [string](Get-Prop $cv 'EditionID')
        display = [string](Get-Prop $cv 'DisplayVersion')
    }
}

# Unpack a bundle into $BinDir (data files into $DataOut\<test>\); returns
# the bundle's id (the first 12 hex digits of the SHA-256 of its text, as
# `pipeline win bundle` names it).
function Expand-Bundle([string[]] $Parts, [string] $BinDir, [string] $DataOut) {
    $all = New-Object IO.MemoryStream
    foreach ($part in $Parts) {
        $bytes = [IO.File]::ReadAllBytes((Resolve-UserPath $part))
        $all.Write($bytes, 0, $bytes.Length)
    }
    $raw = $all.ToArray()
    $id = (Get-Sha256Hex $raw).Substring(0, 12)
    $text = [Text.Encoding]::ASCII.GetString($raw)
    $lines = @($text -split "`r?`n" | Where-Object { $_ -ne '' })
    if ($lines.Count -eq 0 -or $lines[0] -ne $BundleFormat) {
        throw "not a $BundleFormat bundle"
    }
    $files = 0
    $ended = $false
    for ($i = 1; $i -lt $lines.Count; $i++) {
        if ($ended) { throw 'text after the end line' }
        $f = $lines[$i] -split ' '
        switch ($f[0]) {
            'manifest' { }
            'file' {
                if ($f.Count -ne 4) { throw "bad file line $i" }
                $name = $f[1]
                if ($name -notmatch '^[a-z_][a-z0-9_]*\.exe$') { throw "bad file name $name" }
                $bytes = [Convert]::FromBase64String($f[3])
                if ((Get-Sha256Hex $bytes) -ne $f[2]) { throw "$name`: hash mismatch (damaged in transfer?)" }
                [IO.File]::WriteAllBytes((Join-Path $BinDir $name), $bytes)
                $files++
            }
            'data' {
                # data <test> <name> <sha256> <base64> (ADR 0088)
                if ($f.Count -ne 5) { throw "bad data line $i" }
                $test = $f[1]
                $name = $f[2]
                if ($test -notmatch '^[a-z_][a-z0-9_]*$') { throw "bad data test name $test" }
                if ($name -notmatch '^[A-Za-z0-9_][A-Za-z0-9_.-]*$') { throw "bad data file name $name" }
                $bytes = [Convert]::FromBase64String($f[4])
                if ((Get-Sha256Hex $bytes) -ne $f[3]) { throw "$test/$name`: hash mismatch (damaged in transfer?)" }
                $dir = Join-Path $DataOut $test
                if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
                [IO.File]::WriteAllBytes((Join-Path $dir $name), $bytes)
                $script:BundleHasData = $true
                $files++
            }
            'end' {
                if ([int]$f[1] -ne $files) { throw "the end line says $($f[1]) files, the bundle has $files" }
                $ended = $true
            }
            default { throw "unknown bundle line $i" }
        }
    }
    if (-not $ended) { throw 'no end line: the bundle is incomplete' }
    return $id
}

# One run of one test: a hashtable of what happened.
function Invoke-TestOnce([string] $Exe, [string] $Name, [int] $Index) {
    $scratch = Join-Path $Work ("run\$Name.$Index")
    if (Test-Path -LiteralPath $scratch) { Remove-Item -LiteralPath $scratch -Recurse -Force }
    New-Item -ItemType Directory -Path $scratch | Out-Null
    $copy = Join-Path $scratch "$Name.exe"
    $r = @{ status = 'ran'; exit_code = $null; output = $null; stdout = [byte[]]@(); stderr = [byte[]]@(); error = $null; ms = 0 }
    try {
        # Fails if an antivirus has taken the file away meanwhile.
        Copy-Item -LiteralPath $Exe -Destination $copy
        if ($DataDir) {
            $data = Join-Path $DataDir $Name
            if (Test-Path -LiteralPath $data) {
                Get-ChildItem -LiteralPath $data -File | ForEach-Object {
                    Copy-Item -LiteralPath $_.FullName -Destination $scratch
                }
            }
        }
    } catch {
        $r.status = 'error'
        $r.error = 'could not copy it: ' + $_.Exception.Message
        return $r
    }

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $copy
    $psi.WorkingDirectory = $scratch
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        [void]$p.Start()
    } catch {
        $r.status = 'error'
        $r.error = 'could not start: ' + $_.Exception.Message
        return $r
    }
    $p.StandardInput.Close()
    $outStream = New-Object IO.MemoryStream
    $errStream = New-Object IO.MemoryStream
    $outCopy = $p.StandardOutput.BaseStream.CopyToAsync($outStream)
    $errCopy = $p.StandardError.BaseStream.CopyToAsync($errStream)
    $limit = $TimeoutSeconds
    if ($Timeouts.ContainsKey($Name)) { $limit = $Timeouts[$Name] }
    if (-not $p.WaitForExit($limit * 1000)) {
        try { $p.Kill() } catch { }
        [void]$p.WaitForExit(5000)
        $r.status = 'timeout'
    }
    [void]$outCopy.Wait(5000)
    [void]$errCopy.Wait(5000)
    $watch.Stop()
    $r.ms = [int64]$watch.ElapsedMilliseconds
    if ($r.status -eq 'ran') {
        # ExitCode is an Int32; the NTSTATUS-wide value is its bits.
        $r.exit_code = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int32]$p.ExitCode), 0)
    }
    $p.Dispose()
    $r.stdout = $outStream.ToArray()
    $r.stderr = $errStream.ToArray()
    $obs = Join-Path $scratch $ObservationFile
    if (Test-Path -LiteralPath $obs) {
        $r.output = [IO.File]::ReadAllBytes($obs)
    }
    if (-not $KeepScratch) {
        Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
    }
    return $r
}

function Get-RunKey($r) {
    $o = if ($null -eq $r.output) { '-' } else { [Convert]::ToBase64String($r.output) }
    return '{0}|{1}|{2}|{3}|{4}' -f $r.status, $r.exit_code, $o, [Convert]::ToBase64String($r.stdout), [Convert]::ToBase64String($r.stderr)
}

# --- main -------------------------------------------------------------------

# `powershell -File` passes "a,b" as one string: split lists on commas.
$Bundle = @($Bundle | Where-Object { $_ } | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$Tests = @($Tests | Where-Object { $_ } | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$Timeouts = @{}
foreach ($t in @($TestTimeouts | Where-Object { $_ } | ForEach-Object { $_ -split ',' } | Where-Object { $_ })) {
    $kv = $t -split '=', 2
    if ($kv.Count -ne 2 -or $kv[1] -notmatch '^[0-9]+$') { throw "-TestTimeouts: name=seconds, not $t" }
    $Timeouts[$kv[0]] = [int]$kv[1]
}
$script:BundleHasData = $false
if ($Bundle.Count -eq 0 -and -not $TestsDir) {
    throw 'give -Bundle <file or parts> or -TestsDir <directory of .exe files>'
}
if ($Runs -lt 1) { $Runs = 1 }
$Work = Resolve-UserPath $Work
$OutPath = Resolve-UserPath $Out
if (Test-Path -LiteralPath $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
$BinDir = Join-Path $Work 'bin'
New-Item -ItemType Directory -Path $BinDir | Out-Null

$bundleId = $null
if ($DataDir) { $DataDir = Resolve-UserPath $DataDir }
if ($Bundle.Count -gt 0) {
    $dataOut = Join-Path $Work 'data'
    $bundleId = Expand-Bundle $Bundle $BinDir $dataOut
    Write-Host "bundle $bundleId unpacked, hashes good"
    if ($script:BundleHasData -and -not $DataDir) { $DataDir = $dataOut }
} else {
    Get-ChildItem -LiteralPath (Resolve-UserPath $TestsDir) -Filter '*.exe' | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $BinDir
    }
}

# No message boxes: a test that fails to load (a missing export) or
# crashes must end with its status, not wait for a click. The error mode is
# inherited by child processes (Learn: SetErrorMode). SEM_FAILCRITICALERRORS
# 0x1, SEM_NOGPFAULTERRORBOX 0x2, SEM_NOOPENFILEERRORBOX 0x8000 (winbase.h).
try {
    Add-Type -Namespace PolyOracle -Name Native -MemberDefinition '[DllImport("kernel32.dll")] public static extern uint SetErrorMode(uint mode);'
    [void][PolyOracle.Native]::SetErrorMode(0x8003)
} catch {
    Write-Warning "SetErrorMode not set ($($_.Exception.Message)); a failing test may show a dialog until its timeout"
}

$started = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
$results = New-Object System.Collections.ArrayList
foreach ($exe in @(Get-ChildItem -LiteralPath $BinDir -Filter '*.exe' | Sort-Object Name)) {
    $name = [IO.Path]::GetFileNameWithoutExtension($exe.Name)
    if ($Tests.Count -gt 0 -and ($Tests -notcontains $name)) { continue }
    $hash = Get-Sha256Hex ([IO.File]::ReadAllBytes($exe.FullName))
    $runsDone = @()
    for ($i = 0; $i -lt $Runs; $i++) {
        $runsDone += , (Invoke-TestOnce $exe.FullName $name $i)
    }
    $first = $runsDone[0]
    $keys = @($runsDone | ForEach-Object { Get-RunKey $_ } | Select-Object -Unique)
    $stable = ($keys.Count -eq 1)
    $outputText = $null
    $outputB64 = $null
    if ($null -ne $first.output) {
        $outputText = [Text.Encoding]::UTF8.GetString($first.output)
        $outputB64 = [Convert]::ToBase64String($first.output)
    }
    $entry = [ordered]@{
        name       = $name
        exe_sha256 = $hash
        status     = $first.status
        exit_code  = $first.exit_code
        output_b64 = $outputB64
        stdout_b64 = [Convert]::ToBase64String($first.stdout)
        stderr_b64 = [Convert]::ToBase64String($first.stderr)
        output     = $outputText
        error      = $first.error
        stable     = $stable
        ms         = $first.ms
    }
    [void]$results.Add($entry)
    $code = if ($null -eq $first.exit_code) { 'none' } else { '0x{0:x}' -f $first.exit_code }
    $note = if ($stable) { '' } else { ' NOT STABLE between runs' }
    $why = if ($first.error) { ": $($first.error)" } else { '' }
    Write-Host ("{0,-28} {1,-8} exit {2}{3}{4}" -f $name, $first.status, $code, $note, $why)
}

$result = [ordered]@{
    format  = $RunFormat
    system  = Get-SystemInfo
    bundle  = $bundleId
    runner  = $RunnerVersion
    started = $started
    timeout_ms = $TimeoutSeconds * 1000
    test_timeouts_s = $Timeouts
    runs    = $Runs
    tests   = @($results.ToArray())
}
$json = ConvertTo-Json -InputObject $result -Depth 6
[IO.File]::WriteAllText($OutPath, $json, (New-Object Text.UTF8Encoding($false)))
Write-Host "$($results.Count) tests; results: $OutPath"
if ($PrintJson) {
    Write-Output $json
}
