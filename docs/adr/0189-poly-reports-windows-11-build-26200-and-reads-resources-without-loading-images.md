# 0189: Poly reports Windows 11 build 26200, and reads resources without loading images

**Status:** Accepted. Windows applications, batch 1 (track `impl-win-apps`):
the everyday Win32 calls the survey of popular Windows programs
(`windows-app-survey`) found them stopped on. Inputs: the spec sheets and
visible tests of `spec-win-apps-1` (`specs/windows/{kernel32,kernelbase,ntdll,
shlwapi,user32}.dll/`), Microsoft Learn's reference pages and the PE format
specification. No third-party source.

## What was there

- `RtlGetVersion`, `GetVersion[Ex]`, the PEB, `KUSER_SHARED_DATA`, the
  registry's `CurrentVersion` key and the version resource of the system
  DLLs said Windows 10, build 19045, update 2006
  ([ADR 0066](0066-nt-registry-hives-in-polys-own-format.md),
  [ADR 0079](0079-our-kernelbase-and-kernel32.md)).
- `LoadLibraryExW` ignored its flags, so a resource-only DLL, or the
  executable asked for as a data file, went through the image loader (imports,
  entry point and all). There was no resource API at all.
- The path helpers of `shlwapi.dll` and the `PathCch*` family were missing,
  `GetEnvironmentVariableA` and the other ANSI forms were fail-only stubs.

## Decisions

### 1. The reported version is Windows 11, build 26200

The spec agent's oracle is a Windows 11 machine (10.0.26200). The visible
test of `RtlGetVersion` (and of `RtlGetNtVersionNumbers`) accepts any build of
22000 or more, and programs choose code paths by build number (22000 is the
line between "Windows 10" and "Windows 11"), so Poly states the build of the
oracle: `poly_rtl::version::BUILD` = 26200, update 9974 (`UBR`,
`DisplayVersion` 25H2 in the default registry and the version resource of the
system DLLs, `10.0.26200.9974`). What does not change: a program without a
compatibility manifest still gets 6.2.9200 from `GetVersionEx` and
`GetVersion` (as on the oracle), `RtlGetVersion` always the real version.

The suite mask `GetVersionEx` shows is 0x300 (single-user terminal services
and personal: a Home edition, as the oracle's); `RtlGetVersion` writes the
suite mask 0, and neither writes more of the structure than Windows does (the
service pack string keeps its tail), because the spec tests record which bytes
changed. `RtlGetVersion` does not check the size field (any size gets the
basic layout, 284 the extended one).

### 2. Path rules are one host-tested module, checked against the recorded tables

`poly_kernelbase::winpath` holds the rules of the 18 `shlwapi` functions and of
`PathCchSkipRoot`, `PathCchCanonicalizeEx` and `PathCchCombineEx`. The
spec agent recorded each function on four tables of 36 to 40 strings; the
integration test `crates/poly-kernelbase/tests/winpath_tables.rs` runs every
recorded line (`tests/data/<Function>.txt`, the visible tests' results) through
the module, so the rules are checked against the observations and not against a
reading of the sheet. Where the sheet's prose and the recorded data differ the
data won (`PathGetDriveNumberW` accepts `\\?\C:`, `PathAppendW` of an empty path
and `\b` gives `b`). A path is `name (\ name)*`; `..` removes the name before it
with its backslash, the root names (`C:`, the two empty names of `\\`, the
`\\?\Volume{guid}` names) are never removed, and a dot segment dropped right
behind the root leaves the root's backslash.

### 3. Data files are mapped in pieces, not loaded

`LOAD_LIBRARY_AS_DATAFILE`, `..._AS_DATAFILE_EXCLUSIVE` and
`..._AS_IMAGE_RESOURCE` make `kernelbase` read the file's headers and the
section that holds the resource directory into a private allocation of the
image's size, at the same relative addresses, and return its base with bit 0 (or
bit 1) set, as Windows does. The resource functions mask those bits and read
the module the same way for all three kinds of handle (a normal image, a data
file, an image resource); an address that is no module gives
`ERROR_RESOURCE_DATA_NOT_FOUND`. Nothing else of the file is read, so
the 240 MB executable of an Electron program costs its `.rsrc`. `FreeLibrary`
releases the mapping. The alternative, running the file through the image
loader as before, would resolve imports and call entry points of a file that
was only asked for its resources; this is the smaller and safer step.

## Consequences

- Poly's default registry, the version resources of its DLLs and the kernel's
  shared page now say 26200; programs that decide by build number take their
  Windows 11 path.
- `GetEnvironmentVariableA` and its neighbours (git, Python, Firefox,
  Chrome's launcher) pass their first call; `SetEnvironmentVariable[AW]` keeps
  the spelling a variable's name was first given and refuses names and values of
  32,767 units.
- `CopyFileW`, which the resource tests use to copy their own image, is
  implemented (copy of the bytes and the attributes).
- Not done: `LoadStringA`, `FindResourceA` and the `...ExW` enumeration
  functions; the search-path flags of `LoadLibraryExW`; `GetModuleFileName`
  and `GetModuleHandleEx` for a data-file handle.
