# msvc-build.ps1: build the C++ test programs that ask for Microsoft's compiler
# (`// poly-cxx: compiler=msvc`, tools/pipeline/src/wincxx.rs, ADR 0199) on the
# oracle machine. Run it where the Visual Studio Build Tools are installed
# (homeserver2: C:\BuildTools); the container that runs `pipeline win build` has
# no Microsoft compiler, so the executables are built here once, copied back
# beside their sources as <name>.exe and checked in (test artefacts; `win build`
# takes them as they are).
#
#   powershell -File msvc-build.ps1 -Src <dir of .cpp> -Support <tests/windows/cxx> -Out <dir>
#       [-Tests a,b] [-BuildTools C:\BuildTools]
#
# For every <Src>\<name>.cpp that names compiler=msvc it writes <Out>\<name>.exe,
# and <Out>\msvc-build.json (the compiler's version and every command line).
# The program is freestanding: no C or C++ runtime of its own (/Zl, /GS-,
# /NODEFAULTLIB, its own entry point), exceptions through /EHsc by default,
# import libraries made here from the .def files beside poly_wtest.h (export
# names only: the DLLs' own names, nothing of Microsoft's libraries), and a
# reproducible link (/Brepro, no debug information). Nothing of Microsoft's is
# linked in: the executable only imports from vcruntime140.dll,
# vcruntime140_1.dll, msvcp140.dll, ucrtbase.dll and kernel32.dll.
param(
    [Parameter(Mandatory = $true)][string]$Src,
    [Parameter(Mandatory = $true)][string]$Support,
    [Parameter(Mandatory = $true)][string]$Out,
    [string[]]$Tests = @(),
    [string]$BuildTools = 'C:\BuildTools'
)
$ErrorActionPreference = 'Stop'
$Allowed = @('/EHsc', '/EHa', '/EHs', '/O1', '/O2', '/Od', '/GR', '/GR-')   # = wincxx::ALLOWED_CL_FLAGS

# The x64 tools' environment (INCLUDE, LIB, PATH).
$vcvars = Join-Path $BuildTools 'VC\Auxiliary\Build\vcvars64.bat'
cmd /c "`"$vcvars`" >nul 2>&1 && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] }
}
$null = New-Item -ItemType Directory -Force $Out, (Join-Path $Out 'lib'), (Join-Path $Out 'obj')
$clVersion = (cmd /c "cl 2>&1" | Select-Object -First 1)

# Import libraries from the .def files (the stems of the DLLs).
foreach ($def in Get-ChildItem (Join-Path $Support '*.def')) {
    $lib = Join-Path $Out ("lib\" + $def.BaseName + '.lib')
    & lib /nologo /machine:x64 "/def:$($def.FullName)" "/out:$lib" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "lib failed for $($def.Name)" }
}
# msvcp140.dll has about 3,800 exports: its import library is made from the DLL's own export table
# (dumpbin /exports: the names, which are ABI facts) unless a msvcp140.def sits beside poly_wtest.h.
foreach ($dll in 'msvcp140') {
    if (Test-Path (Join-Path $Support "$dll.def")) { continue }
    $sys = Join-Path $env:SystemRoot "System32\$dll.dll"
    if (-not (Test-Path $sys)) { continue }
    $names = dumpbin /nologo /exports $sys | ForEach-Object {
        if ($_ -match '^\s+\d+\s+[0-9A-F]+\s+(?:[0-9A-F]{8}\s+)?(\S+)') { $Matches[1] }
    }
    $def = Join-Path $Out "lib\$dll.def"
    @("LIBRARY $dll.dll", 'EXPORTS') + ($names | ForEach-Object { "    $_" }) | Set-Content $def -Encoding ASCII
    & lib /nologo /machine:x64 "/def:$def" "/out:$(Join-Path $Out "lib\$dll.lib")" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "lib failed for $dll" }
}
# The libraries a program may name, in link order.
$link = @('vcruntime140', 'vcruntime140_1', 'msvcp140', 'ucrtbase', 'kernel32') | ForEach-Object { Join-Path $Out "lib\$_.lib" } |
    Where-Object { Test-Path $_ }

$log = @()
$failed = @()
foreach ($cpp in Get-ChildItem (Join-Path $Src '*.cpp')) {
    $name = $cpp.BaseName
    if ($Tests.Count -gt 0 -and $Tests -notcontains $name) { continue }
    $line = Select-String -Path $cpp.FullName -Pattern 'poly-cxx:(.*)$' | Select-Object -First 1
    if (-not $line -or $line.Matches[0].Groups[1].Value -notmatch 'compiler=msvc') { continue }
    $opts = @()
    if ($line.Matches[0].Groups[1].Value -match 'cl=(\S+)') {
        foreach ($f in $Matches[1].Split(',')) {
            if ($Allowed -notcontains $f) { throw "${name}: cl flag $f is not allowed" }
            $opts += $f
        }
    }
    if (-not ($opts | Where-Object { $_ -like '/EH*' })) { $opts += '/EHsc' }
    if (-not ($opts | Where-Object { $_ -match '^/O' })) { $opts += '/O1' }
    $obj = Join-Path $Out "obj\$name.obj"
    $exe = Join-Path $Out "$name.exe"
    $clArgs = @('/nologo', '/c', '/std:c++17', '/GS-', '/Zl', '/Oy-', '/Brepro', '/utf-8', '/W3', "/I$Support",
                '/DPOLY_CXX_RUNTIME_VCRUNTIME140') + $opts + @("/Fo$obj", $cpp.FullName)
    & cl @clArgs
    if ($LASTEXITCODE -ne 0) { $failed += $name; continue }
    $lnkArgs = @('/NOLOGO', '/NODEFAULTLIB', '/ENTRY:mainCRTStartup', '/SUBSYSTEM:CONSOLE', '/Brepro', '/DEBUG:NONE',
                 '/MACHINE:X64', "/OUT:$exe", $obj) + $link
    & link @lnkArgs
    if ($LASTEXITCODE -ne 0) { $failed += $name; continue }
    $log += [ordered]@{ test = $name; cl = ($clArgs -join ' '); link = ($lnkArgs -join ' ');
                        sha256 = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower() }
}
[ordered]@{ compiler = $clVersion; tests = $log } | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $Out 'msvc-build.json') -Encoding UTF8
"built $($log.Count) test programs with: $clVersion"
if ($failed.Count -gt 0) { "FAILED: $($failed -join ' ')"; exit 1 }
