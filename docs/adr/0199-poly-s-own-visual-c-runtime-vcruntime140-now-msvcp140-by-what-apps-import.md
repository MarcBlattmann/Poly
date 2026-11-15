# 0199: Poly's own Visual C++ runtime: `vcruntime140` now, `msvcp140` in the order apps import it, nothing of Microsoft's

**Status:** Accepted (owner decision, 2026-11-15: "Poly ships its own Visual C++ runtime, do
it if it makes sense"). Takes up the fallback of [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md)
for the Visual C++ 2015-and-later runtime; ADR 0055 stays in force for every other
redistributable. Builds on [ADR 0098](0098-cpp-windows-tests-and-the-cpp-exception-spec.md)
(C++ test programs) and ADR 0099 (C++ exceptions in Poly's `msvcrt`), [ADR 0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)
(fail-only exports, `polystub.dll`) and [ADR 0186](0186-32-bit-windows-programs-compatibility-mode-poly-s-own-i686-ntdll-and-converted-system-calls.md)
(32-bit programs). Some parts need the owner's yes: see "Owner decisions needed".

## Context

**What Windows does.** The Visual C++ runtime is not part of Windows. Windows has the
Universal C runtime (`ucrtbase.dll`, a system component since Windows 10) and `msvcrt.dll`;
the C++ side of Visual C++ 2015 to 2022 (one binary-compatible "v14" family) is three groups
of DLLs that come with an application or with Microsoft's installer `vc_redist.x64.exe` /
`vc_redist.x86.exe`:

| DLL | Exports (map) | What it is |
|---|---:|---|
| `vcruntime140.dll` | 88 (74 on x64) | C++ exceptions and RTTI, `type_info`, `std::exception` helpers, `memcpy` and friends, `setjmp`/`longjmp`, `/GS` failure |
| `vcruntime140_1.dll` | 3 | `__CxxFrameHandler4`, the handler of the compressed ("FH4") exception tables Visual C++ 2019+ emits on x64 by default |
| `msvcp140.dll` | 3,758 | the separately compiled part of the C++ standard library: streams, locales and facets, threads and mutexes, `std::exception_ptr`, the `_X*` throw helpers |
| `msvcp140_1`, `_2`, `_atomic_wait`, `_codecvt_ids` | 7 / 88 / 32 / 4 | memory resources, special math functions, `atomic::wait`, `codecvt` ids |
| `concrt140.dll` | 727 | the Concurrency Runtime (PPL tasks, agents) |

ADR 0055 made all of them `app-provided`: the application brings them. That fails for apps
that *expect* the redistributable to be installed. The app survey (handoffs
`windows-app-survey`, `win-apps-exports`) and a scan of the import tables of five downloads for this ADR
(names only) show both patterns:

| App | Imports `vcruntime140` / `_1` / `msvcp140*` | Ships them |
|---|---|---|
| KeePassXC 2.8.0 (Qt 6) | 23 / 1 / 151 + 1 + 1 names, from 31 modules | **no** (ships `vc_redist.x64.exe` for its installer) |
| LibreOffice 26.8.3 (MSI, flat extraction) | yes | no (a merge module in the MSI) |
| XnView MP 1.12.3 (Qt 6) | 33 / 1 / 268 + 1 + 2 | yes, app-local |
| Paint.NET 5.2 | 23 / 1 / 39 | yes, app-local |
| Python 3.13.9 embeddable | 20 / 1 / 0 | yes (`vcruntime140*.dll` only) |

Every C++ program built with Visual C++ 2015 or later imports `vcruntime140`; programs built
with Visual C++ 2019 or later for x64 also import `vcruntime140_1` (FH4). Poly's `msvcrt.dll`
already handles C++ exceptions with `__CxxFrameHandler3`/`__CxxFrameHandler` (ADR 0099), and
`vcruntime140`'s `_CxxThrowException` and `__CxxFrameHandler3` have the same contract: the
same exception record, the same `FuncInfo` tables, the same throw descriptors.

## Options

**(a) Clean-room Rust implementations of the exported names.** Matches what Poly does for
every system DLL. The cost differs by DLL:

- `vcruntime140` is small and its hard part exists: the exception protocol is specified
  (`specs/windows/msvcrt.dll/{__CxxFrameHandler3,__CxxFrameHandler,_CxxThrowException}.yaml`)
  and implemented (`poly-cxxeh`). What is new is RTTI (`__RTDynamicCast`, `__RTtypeid`: the
  complete-object locator and class hierarchy descriptors the compiler emits beside a
  virtual table), the name undecorator (`__unDName`, `type_info::name`) and FH4.
- `msvcp140` is large and its difficulty is the ABI, not the algorithms: the program's own
  inlined template code (from Visual C++'s headers) reads and writes the fields of
  `basic_streambuf`, `basic_ios`, `ios_base`, `locale::_Locimp`, the facets and `_Mtx_t`
  directly, calls their virtual functions through virtual tables laid out by the headers,
  and uses exported *data* (`std::locale::id::_Id_cnt`, `std::ctype<char>::id`,
  `std::cout`). Each exported class must have exactly the size, field offsets, virtual table
  order and RTTI that the headers give, and each member function the behaviour the headers'
  inline code expects of it.
- FH4's compressed tables are not publicly documented and clang cannot emit them; only
  programs built by Microsoft's compiler have them.

**(b) Reuse Microsoft's Apache-2.0 STL sources** (`github.com/microsoft/STL`, Apache-2.0
WITH LLVM-exception). They contain the headers and the separately compiled sources of
`msvcp140` (not `vcruntime`, whose sources ship only with Visual Studio under its licence).
Against it: Poly's build is pure Rust with `rust-lld` and must build the same on Windows and
Linux hosts (ADR 0124); compiling the STL needs a C++ compiler that targets the MSVC ABI
(clang-cl or MSVC), its headers and the Universal CRT headers; `CLEAN_ROOM.md` forbids
copying code from any project into the repository, whatever its licence; and code built
from those sources would carry Apache-2.0 notices into Poly's images. It would give
`msvcp140` exactly, quickly. It needs the owner (licence and clean-room rule).

**(c) Let the user run the official `vc_redist` installer on Poly.** Microsoft's licence lets
an application's developer redistribute the runtime with the application and lets the user
install it; it does not let Poly ship, download or patch Microsoft's files (ADR 0055).
It needs Windows Installer, Burn bundles and `System32`/`SysWOW64` writes to work on Poly
(not yet), and an app that runs without an installer (KeePassXC's zip, LibreOffice's flat
extraction) still fails until the user finds and runs it. It is a user option, not Poly's
answer.

**(d) Hybrid.** Poly's own clean-room `vcruntime140` and `vcruntime140_1` now (small, the
exception code exists); `msvcp140` and its siblings in the order of the names apps import,
by the normal spec and implementer pipeline, fail-only for the rest (a missing name in a
present DLL is a logged failure, not a process that cannot start); the application's own
copies still win; the user may still install Microsoft's runtime.

## Decision

**Option (d).** It keeps Poly's build and clean-room rules as they are, gets every Visual
C++ program past its first import at once, puts the work where apps need it, and lets a
program that brings Microsoft's DLLs keep using them.

### Where the DLLs live and who wins

- `C:\Windows\System32\vcruntime140.dll`, `vcruntime140_1.dll` (and later `msvcp140*.dll`),
  installed by the kernel's layout like every Poly DLL (`kernel/src/nt/layout.rs`), built in
  the Win32 workspace (`personalities/nt/win32/{vcruntime140,vcruntime140_1}`). The 32-bit
  copies go to `C:\Windows\SysWOW64` with the i686 build (ADR 0186), below.
- The loader's search order is Windows': the application's directory before `System32`.
  A program that ships Microsoft's DLLs (XnView MP, Paint.NET, Python) loads its own; one
  that does not (KeePassXC) gets Poly's. Nothing is redirected or replaced.
- They are not "known DLLs": an application-local copy must stay possible.

### `vcruntime140.dll` (x64, built now)

| Exports | How |
|---|---|
| `_CxxThrowException`, `__CxxFrameHandler3`, `__CxxFrameHandler2`, `__CxxFrameHandler` | `msvcrt`'s code (ADR 0099), moved into the crate `poly-win32-cxxrt` that both DLLs link; `terminate()` is `ucrtbase`'s, as the Universal CRT's `set_terminate` sets it |
| `__current_exception`, `__current_exception_context`, `__processing_throw`, `__uncaught_exception(s)` | per-thread slots kept by the catch machinery: the record and context of the innermost running catch block's exception; the count of exceptions thrown and not yet caught (C++17 [uncaught.exceptions]) |
| `__DestructExceptionObject`, `__CxxDetectRethrow`, `_is_exception_typeof`, `__AdjustPointer` | from the specified record and throw-descriptor layouts (`_CxxThrowException.yaml`) |
| `__std_type_info_compare`, `__std_type_info_hash`, `__std_type_info_destroy_list`, `__std_exception_copy`, `__std_exception_destroy` | the public headers' structures (`vcruntime_typeinfo.h`, `vcruntime_exception.h`), behaviour by the C++ standard; host-tested in `poly_cxxeh::typeinfo` |
| `set_unexpected`, `_get_unexpected`, `unexpected`, `_set_se_translator`, `__report_gsfailure`, `_local_unwind`, `__telemetry_*`, `_SetWinRTOutOfMemoryExceptionCallback`, `__GetPlatformExceptionInfo`, `_NLG_*` | Learn's descriptions (handlers kept process-wide for now, the translator not called yet, `/GS` failure a fail-fast with `FAST_FAIL_STACK_COOKIE_CHECK_FAILURE`) |
| `memcpy` … `wcsstr`, `__intrinsic_setjmp(ex)`, `longjmp`, `_purecall`, `_set/_get_purecall_handler`, `__std_terminate` | forwarders to `ucrtbase` (same contract, one implementation) |
| `__C_specific_handler`, `__C_specific_handler_noexcept` | forwarders to `ntdll` |
| `__vcrt_GetModuleFileNameW`, `__vcrt_GetModuleHandleW`, `__vcrt_LoadLibraryExW`, `__vcrt_InitializeCriticalSectionEx` | forwarders to `kernel32` |
| `__RTDynamicCast`, `__RTtypeid`, `__RTCastToVoid`, `__unDName`, `__unDNameEx`, `__BuildCatchObject(Helper)`, `__TypeMatch`, `__FrameUnwindFilter`, `__CxxExceptionFilter`, `__CxxQueryExceptionSize`, `__CxxRegister/UnregisterExceptionObject`, `_CreateFrameInfo`, `_FindAndUnlinkFrame`, `_IsExceptionObjectToBeDestroyed` | fail-only (0 / NULL, logged `stub: vcruntime140!name`) until their spec sheets exist; `__std_type_info_name` gives the decorated name (logged) until the undecorator exists |

### `vcruntime140_1.dll` (x64, built now)

`__CxxFrameHandler4` logs `stub: vcruntime140_1!__CxxFrameHandler4` and continues the search:
programs that throw nothing run; a throw through an FH4 frame is neither caught nor
unwound there (what Poly's `msvcrt` did before ADR 0099), and the log shows it. The real
handler waits for the FH4 spec (below). `__NLG_Dispatch2`, `__NLG_Return2`: debugger hooks
that return.

### `msvcp140.dll` and siblings: by what apps import

The next implementer rounds take `msvcp140` names in the order of how many apps and modules
import them (the ranked list is in the hand-back `vcrt-design.md`; the first groups):

1. **The throw helpers** `_Xlength_error`, `_Xout_of_range`, `_Xbad_alloc`,
   `_Xbad_function_call`, `_Xinvalid_argument`, `_Throw_Cpp_error`, `_Syserror_map`,
   `_Winerror_map` (every app; 63 modules call `_Xlength_error`). They throw the standard
   exception classes, so `msvcp140` needs their virtual tables, RTTI and throw descriptors.
2. **Locks and the locale core**: `_Lockit`, `locale::_Getgloballocale`, `locale::_Init`,
   `locale::classic`, `locale::id::_Id_cnt` and the facet ids (`ctype<char>::id`,
   `codecvt<char,char,_Mbstatet>::id`, `numpunct<char>::id`: exported *data*), `_Getcat`,
   `facet::_Incref/_Decref`, `_Locinfo`.
3. **`ios_base` / `basic_ios` / `basic_streambuf` / `basic_[io]stream` members** for `char`
   (`good`, `flags`, `width`, `setstate`, `clear`, `sputc`, `sputn`, `xsputn`, `_Osfx`,
   `flush`, `operator<<` ...), then `wchar_t`, then `std::cout` and friends.
4. **Threads**: `_Thrd_hardware_concurrency`, `_Mtx_*`, `_Cnd_*`, `_Query_perf_counter`,
   `_Query_perf_frequency`, `_Xtime_get_ticks` (on SRW locks and condition variables).
5. **`std::exception_ptr`** (`__ExceptionPtrCreate/Copy/Destroy/Assign/Rethrow/ToBool/
   CurrentException`, `uncaught_exception(s)`), on `vcruntime140`'s current exception.
6. `msvcp140_1!_Aligned_get_default_resource`, `msvcp140_2!__std_smf_*`, the
   `Concurrency::details` task helpers (`_Schedule_chore`, `_TaskEventLogger`), then the rest.

A name not yet implemented is fail-only, generated as for the other DLLs (ADR 0155, step 10);
an exported data item that is not yet real is a zeroed object of the documented size. **Class
layouts** (sizes, field offsets, virtual table order) and exported data come from spec
sheets: a spec agent compiles probe programs against Visual C++'s headers on the Windows
oracle and records `sizeof`/`offsetof`/virtual table order and behaviour, and may read the
Apache-2.0 STL headers for behaviour as `CLEAN_ROOM.md` allows for permissively licensed code
(owner confirmation below). Implementers get the sheets, never the headers.

### Coverage map

- `vcruntime140.dll`, `vcruntime140_1.dll`: no longer `app-provided`. Every x64 name this ADR
  implements is `claimed` by `vcrt-design` (not green: no spec sheet of these DLLs exists
  yet); the fail-only ones and `__CxxFrameHandler4` are `stub`; the i386-only names are
  `missing` (the 32-bit step).
- `msvcp140.dll`, `msvcp140_1.dll`, `msvcp140_2.dll`, `msvcp140_atomic_wait.dll`,
  `msvcp140_codecvt_ids.dll`: `missing` (ours to build, in the order above).
- `concrt140.dll`, the older `msvcp*`/`msvcr*` versions, `vcomp*`, the D3DX and DirectX
  end-user runtime: stay `app-provided` (ADR 0055); `concrt140` is an owner decision below.

### How behaviour is observed

- **C++ test programs (ADR 0098)** can take their runtime from `vcruntime140.dll`:
  `// poly-cxx: runtime=vcruntime140`, or `pipeline win build --cxx-runtime vcruntime140`
  for a whole build. The import libraries come from `tests/windows/cxx/vcruntime140.def`
  and `ucrtbase.def` (export names only), listed before `kernel32` and `msvcrt`, and
  `poly_wtest.h` takes `terminate`/`set_terminate` from `ucrtbase` then. A build for `msvcrt`
  is unchanged byte for byte. The executables import only `ntdll`, `kernel32`, `msvcrt`,
  `vcruntime140` and `ucrtbase`, so the same file runs on Windows (with the redistributable
  installed on the oracle) and on Poly.
- **`pipeline win polycheck`** (CI) runs the C++ tests of the green `msvcrt.dll` entries a
  second time built for `vcruntime140.dll` and compares them with the results recorded for
  `msvcrt` on Windows: the two DLLs share the code, and until a spec agent records
  `vcruntime140`'s own results this checks that it behaves as the specified `msvcrt` does.
  The implementer's own tests of the new names (`tests/windows/vcrt_exception_state.cpp`,
  `vcrt_typeinfo_exception.cpp`: self-checking, expected values from the C++ standard) run
  in the same boot and must pass.
- **Spec work next** (spec agents, Windows oracle with the redistributable installed):
  `vcruntime140.dll` sheets (re-record the frame handler tests against it; RTTI with clang's
  own RTTI output: `dynamic_cast`, `typeid`, multiple and virtual inheritance;
  `type_info::name` and `__unDName` outputs; `__std_type_info_hash` values;
  `uncaught_exceptions`, `__current_exception`), then FH4, then the `msvcp140` groups.
- **FH4** needs programs built by Microsoft's compiler (`cl.exe /EHsc`, FH4 is its x64
  default): a spec agent builds test programs with it on the oracle as a black box and
  specifies the tables from what the compiler emits for chosen inputs and how
  `__CxxFrameHandler4` behaves on them. That needs Visual Studio or the Build Tools on the
  oracle (owner decision below). The pipeline would then accept checked-in `.exe` test
  artefacts for FH4 entries (built on Windows, recorded by hash), since the container has
  no Microsoft compiler.

### 32-bit (WOW64)

Not built in this step: x86 C++ exceptions are frame-based (`fs:[0]`, `__CxxFrameHandler3`
with the `FuncInfo` in `EAX`, `_EH_prolog`, `_except_handler4_common`, `_local_unwind4`,
`_setjmp3`), a different design from x64's table-driven one, and the i686 `ucrtbase` that
`vcruntime140` imports does not exist yet (ADR 0186's staged C runtime). Order: i686
`ucrtbase` + `msvcrt`, then the i686 `vcruntime140` (a frame-based `poly-cxxeh` front end
over the same matching rules), installed in `SysWOW64`.

## Owner decisions needed

1. **Reusing Microsoft's STL sources (option b).** Not recommended: it needs a C++ compiler in
   Poly's build (against ADR 0124's pure-Rust rule), changes `CLEAN_ROOM.md`'s "no copying
   code from any project" rule, and puts Apache-2.0 code (with its notices) into Poly's
   images. Only with the owner's yes, and then as its own ADR.
2. **Spec agents reading the Apache-2.0 STL headers** for class layouts and the behaviour of
   `msvcp140`'s members, as `CLEAN_ROOM.md` already allows for permissively licensed code
   (implementers never see them). Recommended yes; this is the first Microsoft open-source
   project used that way, so it is put to the owner.
3. **Microsoft's compiler on the Windows oracle** (Visual Studio Community or Build Tools,
   under Microsoft's licence to the owner) for FH4 and later `msvcp140` probes, used as a
   black box to build test programs; and checked-in FH4 test executables. Recommended yes.
4. **`concrt140.dll`**: keep `app-provided` (recommended: few apps import it, those seen
   ship it) or build it.
5. **A user-installed `vc_redist`** (when Windows Installer runs on Poly): Microsoft's
   installer would replace Poly's copies in `System32` with Microsoft's, under Microsoft's
   licence to the user. Recommended: allow it (Poly never ships, fetches or patches the
   files; ADR 0055's rule stands), and give Poly's DLLs a lower file version than any
   Microsoft release so the installer's version check replaces them rather than refusing.

## Consequences

- Every Visual C++ 2015+ program finds `vcruntime140.dll` and `vcruntime140_1.dll`; its C++
  exceptions work as `msvcrt`'s do, except through FH4 frames (logged).
- `msvcrt.dll`'s C++ exception code moved, unchanged in behaviour, into
  `personalities/nt/win32/cxxrt` (`poly-win32-cxxrt`); it gained the per-thread current
  exception and uncaught count, which `msvcrt` exports nothing for.
- `msvcp140` and its siblings count as Poly's work in the coverage percentages from now on;
  the Windows denominator grows by 3,889 entries (and 91 for the two `vcruntime` DLLs).
- KeePassXC, LibreOffice's flat extraction and Qt apps without app-local runtimes move from
  "missing DLL" to "missing `msvcp140` names", which the ranked list orders.
- `win polycheck` boots one more time (the `vcruntime140` pass).
