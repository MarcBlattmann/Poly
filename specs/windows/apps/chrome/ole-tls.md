# Chrome's component updater and COM: the apartment in `TEB.ReservedForOle`, the global interface table, BITS

Spec side (agent `spec-com-ole-tls`) for the implementer `impl-com`. Observed behaviour of real Windows only (CLEAN_ROOM.md):
test programs of our own on the oracle machine (`homeserver2`, Windows 11 25H2 build 26200.9974, session 0 over SSH, x64;
every test ran twice with identical output), reading data of their own threads. Nothing of Chrome or of any Microsoft binary was
read, decompiled or disassembled. Entries and tests:

| Entry | Sheet | Tests (visible / held out) |
|---|---|---|
| `windows/api/ole32.dll/TEB.ReservedForOle` | `specs/windows/ole32.dll/TEB.ReservedForOle.yaml` | 4 / 2 |
| `windows/api/combase.dll/RoInitialize`, `RoUninitialize` | `specs/windows/combase.dll/` (pointer sheets) | called by the tests above |
| `windows/api/ole32.dll/CLSID_StdGlobalInterfaceTable` | `specs/windows/ole32.dll/CLSID_StdGlobalInterfaceTable.yaml` | 2 / 1 |
| `windows/api/ole32.dll/IGlobalInterfaceTable.{Register,Get,Revoke}InterfaceInGlobal`/`FromGlobal` | pointer sheets in `specs/windows/ole32.dll/` | see the class entry |
| `windows/api/ole32.dll/CLSID_BackgroundCopyManager` | `specs/windows/ole32.dll/CLSID_BackgroundCopyManager.yaml` | 2 / 1 |

## Why

The hunt `fable-c62f809` found that Chrome's component updater starts a BITS download in a task (`DoStartDownload`) whose first
statement reads the thread's apartment straight from `TEB.ReservedForOle` (`winternl.h`; byte offset **0x1758** of the 64-bit
TEB, `gs:[0x30] + 0x1758`) and the 32-bit value at offset **+0x14** of the block that field points to. Poly's `ole32` keeps the
apartment elsewhere and never sets the field, so the task stops at its check. After it comes a request for the global interface
table and for BITS. The tests below show what Windows has in the field in every state, so Poly can produce the same.

## 1. `TEB.ReservedForOle` (x64: pointer at TEB+0x1758)

Block pointer `p`; "value" = 32-bit value at `p+0x14`; count = at `p+0x28` (initialisations not yet undone); ole = at `p+0x2c`
(OleInitialize calls not yet undone). All numbers hexadecimal. `CoGetApartmentType` in the same state: hr / type / qualifier.

| State of the thread | `p` | value | count | ole | `CoGetApartmentType` |
|---|---|---|---|---|---|
| new thread, no COM call; also the main thread before any call | **NULL** | | | | 0x800401F0 |
| `CoInitializeEx(NULL, 2)` first call (S_OK) | set | **0x81** | 1 | 0 | 0 / 3 (first STA of the process; 0 for a later STA) / 0 |
| `CoInitializeEx(NULL, 6)` (STA + DISABLE_OLE1DDE; Chrome's UI thread) | set | **0xC1** | 1 | 0 | 0 / 3 (or 0) / 0 |
| `CoInitializeEx(NULL, 0xA)` (STA + SPEED_OVER_MEMORY) / `0xE` | set | 0x81 / 0xC1 | 1 | 0 | 0 / 3 / 0 |
| `CoInitializeEx(NULL, 0)` (MTA) | set | **0x141** | 1 | 0 | 0 / 1 / 0 |
| `CoInitializeEx(NULL, 4)` (MTA + DISABLE_OLE1DDE; Chrome's other threads), `8`, `0xC` | set | **0x141** | 1 | 0 | 0 / 1 / 0 |
| nested call, same model (S_FALSE) | same block | unchanged | 2 | 0 | unchanged |
| other model on an initialised thread (0x80010106) | same block | unchanged | unchanged | 0 | unchanged |
| after one `CoUninitialize` of two | same block | unchanged | 1 | 0 | unchanged |
| after the last matching `CoUninitialize` | **same block, not cleared** | **0x1** | 0 | 0 | 0x800401F0, outputs untouched |
| extra `CoUninitialize` | same | 0x1 | 0 | 0 | 0x800401F0 |
| initialised again after that, either model | **same block as before** | as the rows above | 1 | 0 | as above |
| `OleInitialize(NULL)` on a new thread | set | 0x81 | 1 | 1 | 0 / 3 / 0 |
| nested `OleInitialize` | same | 0x81 | 2 | 2 | |
| `OleInitialize` after `CoInitializeEx(2)` | same | 0x81 | 2 | 1 | 0 / 3 / 0 |
| `OleInitialize` on an MTA thread (0x80010106) | same | 0x141 | 1 | 0 | 0 / 1 / 0 |
| `RoInitialize(RO_INIT_SINGLETHREADED)` on a new thread | set | **0x2000C1** | 1 | 0 | 0 / 3 / 0 |
| `RoInitialize(RO_INIT_MULTITHREADED)` | set | **0x200141** | 1 | 0 | 0 / 1 / 0 |
| `RoInitialize` nested (S_FALSE) / other kind (0x80010106) | same | unchanged | 2 | 0 | unchanged |
| two `RoUninitialize` (a third changes nothing) | same | **0x1** | 0 | 0 | 0x800401F0 |
| `RoInitialize(0)` after `CoInitializeEx(2)`: S_FALSE; `RoInitialize(1)`: 0x80010106 | same | stays 0x81 | 2 | 0 | 0 / 3 / 0 |
| **implicit MTA**: a thread that never called `CoInitializeEx` while another thread holds the MTA | **NULL** | | | | **0 / 1 / 1** (qualifier 1) |
| ... after its first COM call that needs the apartment (`CoCreateInstance` of the global interface table, S_OK) | set | **0x1** | 0 | 0 | 0 / 1 / 1 |
| ... after `CoCreateInstance(CLSID_BackgroundCopyManager)` (out-of-process class, S_OK) | set | **0x3** (stays 0x3 after the thread's own `CoUninitialize`) | 0 | 0 | 0 / 1 / 1 |
| a thread that never initialised calls `CoCreateInstance`, `CoGetClassObject` or `CoGetObjectContext` (all 0x800401F0) | **becomes non-NULL** | **0x1** | 0 | 0 | 0x800401F0 |
| a thread that never initialised calls `CoGetApartmentType`, `CoUninitialize`, `CoInitializeEx` with bad flags or a reserved pointer (0x80070057), `OleInitialize(1)`, `CoTaskMemAlloc/Free`, `CLSIDFromString` | stays NULL | | | | 0x800401F0 |

Reading of the bits (inferred from which combinations occur; the numbers above are the contract): 0x1 = the thread has used COM
and never loses it; 0x80 = apartment threaded; 0x100 = multithreaded; 0x40 added to 0x80 by `COINIT_DISABLE_OLE1DDE`
(an MTA call with flag 4 gives 0x141, the same as without); 0x200000 = Windows Runtime initialised; 0x2 = an out-of-process class
was used. Each thread has its own block; a block is allocated once on first use and then reused for the thread's lifetime.
`SPEED_OVER_MEMORY` (8) changes nothing.

The 32-bit layout (observed once, a 32-bit process on the same machine, not in a recorded test because the tests' support
crate is 64-bit): pointer at TEB+**0xF80**; flags value at `p+0x0C`, count at `p+0x18`, ole count at `p+0x1C`; the values for
the same states are the same numbers (0x81, 0xC1, 0x141, 0x1; 0x81 with ole 1 after `OleInitialize`).

**For `ole32` (what the hunt asked for):** on the first call of `CoInitializeEx`, `OleInitialize` or `RoInitialize` allocate a
zeroed block of at least 0x30 bytes (x64) and store it in `TEB.ReservedForOle`; on every initialise/uninitialise update the value
at +0x14 and the counts as the table says; never free or clear it while the thread lives (CoUninitialize leaves 0x1); give the
three failing calls on an uninitialised thread (`CoCreateInstance`, `CoGetClassObject`, `CoGetObjectContext`) the 0x1 block;
leave an implicit-MTA thread NULL until it makes its first call that needs the apartment. Which value Chrome's check accepts is
not known from this sheet: with every state listed, Poly can give exactly what Windows gives, which is what the check was
written against.

## 2. The global interface table

`CoCreateInstance(CLSID_StdGlobalInterfaceTable {00000323-0000-0000-C000-000000000046}, NULL, CLSCTX_INPROC_SERVER (or ALL),
IID_IGlobalInterfaceTable {00000146-0000-0000-C000-000000000046})`: S_OK on an initialised thread, the same table object for
every thread of the process; contexts 0, 4 (local) and 0x10 (remote) give 0x80040154; IID_IMarshal gives 0x80004002; an outer
unknown gives 0x80040110; a thread without COM gives 0x800401F0. Vtable after IUnknown: `RegisterInterfaceInGlobal(object, iid,
&cookie)`, `RevokeInterfaceFromGlobal(cookie)`, `GetInterfaceFromGlobal(cookie, iid, &object)`.

| Call | Result |
|---|---|
| Register (IID_IUnknown) | S_OK, cookie non-zero, object's reference count +1; the same object again: another cookie, +1; the interface id is not checked against the object |
| Get in the registering apartment | S_OK, the very object, +1 (the caller releases); an interface the object lacks: 0x80004002, output NULL, no change |
| Get in another apartment (STA to MTA, MTA to STA, STA to STA) | S_OK, a proxy (not the registered pointer); the registering STA must serve calls |
| Revoke | S_OK, the table's reference released (-1); also from another apartment |
| Get/Revoke of a revoked cookie, or of 0, 1, 0x1234, 0xFFFFFFFF | 0x80070057 (E_INVALIDARG), Get output NULL |
| Get after the registering STA called its last `CoUninitialize` | 0x80070057 |
| a new registration after revokes | a cookie equal to none of the earlier ones |

## 3. BITS

`CoCreateInstance(CLSID_BackgroundCopyManager {4991D34B-80A1-4291-83B6-3328366B9097}, NULL, ctx,
IID_IBackgroundCopyManager {5CE34C0D-0DC9-4C1F-897C-DAA1B78CEE7C})` on the oracle (BITS demand-start, stopped until used):
**S_OK** for CLSCTX_ALL (0x17), LOCAL_SERVER (4) and 5, in STA and MTA threads; **0x80040154** for INPROC_SERVER (1),
REMOTE_SERVER (0x10) and context 0; IID_IGlobalInterfaceTable 0x80004002; outer unknown 0x80040110; thread without COM
0x800401F0; implicit-MTA thread S_OK. A class nothing registers: 0x80040154 in every context (also `CoGetClassObject`, and the
all-zero class id). **Where BITS is not there** the sheet gives no observation (the oracle's service configuration was not changed: that is
the owner's decision). The honest answer Poly can give is the one Windows gives for any class that is not registered:
`REGDB_E_CLASSNOTREG` (0x80040154), output NULL. Whether Chrome falls back to its normal downloader then needs a Chrome run.

## Checking Poly

The visible tests run on Poly with `cargo xtask pipeline win poly --tests-dir specs/windows/ole32.dll/<entry>.tests` and are
compared with `windows.json` beside them; when an entry is green, `cargo xtask pipeline win polycheck` keeps it so.
