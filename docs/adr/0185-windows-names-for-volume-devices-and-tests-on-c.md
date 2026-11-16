# 0185: Disk volumes are `\Device\HarddiskVolume<N>`, and Windows tests run from `C:`

**Status:** Accepted. Replaces the device names of [ADR 0067](0067-nt-file-io.md)
(`\Device\PolyFs<N>`) and the place of the test program in
[ADR 0075](0075-windows-test-oracle.md) (`Z:\test-binary`).

## Context

Every NT call that reports where a file lives gives the device's object name:
`NtQueryObject(ObjectNameInformation)` of a file, `K32GetMappedFileNameW` and
`GetMappedFileNameW`, `GetFinalPathNameByHandleW(VOLUME_NAME_NT)`, the targets of
the `\??\C:` links, the entries of `\Device`. On Windows the name of a disk
volume is `\Device\HarddiskVolume<N>` (the sheets of `NtQueryObject`,
`K32GetMappedFileNameW` and the Chrome traces all show it), and a program and the
system DLLs are usually on one volume, so a test can check that a module and
`ntdll.dll` have the same device prefix.

Poly named its volumes `\Device\PolyFs<N>`, and the Windows test pipeline put the
test program on `Z:` (`\Device\PolyFs0`, the whole VFS) while `ntdll.dll` and
`kernel32.dll` are on `C:` (`\Device\PolyFs1`, `/nt/c`). Both names differed from
Windows', and the two prefixes differed from each other
(`impl-win-b-batch`, open point of `K32GetMappedFileNameW`).

## Decision

1. **The devices are `\Device\HarddiskVolume<N>`.** The system volume `C:`
   (`/nt/c`) is `HarddiskVolume1`, the lowest number; `Z:` (`/`) is
   `HarddiskVolume2`; each other mount gets `HarddiskVolume3`... in mount-path
   order. `personalities/nt/src/io`: `harddisk_volume`, `default_volumes`,
   `IoState::volume_device_path`. Nothing else is named `PolyFs` any more: the
   object namespace, the file object names, mapped file names and the symbolic
   links all follow from the one device object. Windows numbers its volumes from
   whatever its disk layout has (often 3 or 4 for `C:`), and programs may only
   rely on the form; Poly counts from 1.
2. **`GetFinalPathNameByHandleW(VOLUME_NAME_NT)` works**: the file object's name
   (device and path), as `NtQueryObject` gives it. Only `VOLUME_NAME_GUID` stays
   `ERROR_NOT_SUPPORTED`. `QueryDosDeviceW` does not exist on Poly yet; when it
   does it asks the object namespace and so answers the same names.
3. **Windows tests run from `C:\PolyTests`.** `pipeline win poly` puts the test
   executable at `/nt/c/PolyTests/test-binary.exe` (`poly.init=` that path,
   `poly.test.stdout=/nt/c/PolyTests/poly-test.out`), so the program and the system
   DLLs share the volume device. Nothing in the pipeline needs `Z:`: the
   observation file is read by VFS path, and the test support library finds it
   beside the executable.

## Consequences

* The `Z:` letter and `\Device\HarddiskVolume2` still show the whole VFS; a file
  reached through `Z:\nt\c\...` has a different object name than the same file
  through `C:\...` (the spelling of the path used to open it, as before).
* `\Device\PolyFs<N>` no longer opens; the other Poly devices (`PolyPipe`,
  `PolyConsole`, `Afd`) keep their names.
* The Linux test pipeline and the app tests are untouched (`/test-binary`).
