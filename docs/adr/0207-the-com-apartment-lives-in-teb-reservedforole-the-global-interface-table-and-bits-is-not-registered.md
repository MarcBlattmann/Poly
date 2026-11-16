# 0207: The COM apartment lives in `TEB.ReservedForOle`; the global interface table; BITS is not registered

**Status:** Accepted. Chrome for Windows, stage 3 (track `impl-com`): the browser's
window stays up past the component updater's download. Inputs: the spec sheets and
visible tests of `spec-com-ole-tls` (`specs/windows/ole32.dll/TEB.ReservedForOle`,
`CLSID_StdGlobalInterfaceTable`, `CLSID_BackgroundCopyManager`;
`specs/windows/apps/chrome/ole-tls.md`), Microsoft Learn ("CoInitializeEx",
"OleInitialize", "RoInitialize", "CoGetApartmentType", "IGlobalInterfaceTable",
"CoCreateInstance") and `winternl.h` (`TEB.ReservedForOle`). No third-party source.

## What was there

`ole32` kept a thread's apartment in a TLS slot of its own and never set
`TEB.ReservedForOle`. Chrome's component updater reads its COM thread's apartment
straight from the block that field points to (the 32-bit value at +0x14) and stopped
with an `int3` at `chrome.dll+0xc62f809` about 35 s after the first page (hunt
`fable-c62f809`, ADR 0155's "S3 hunt 3").

## Decisions

1. **The block is the one truth.** `ole32` allocates a zeroed 0x40-byte block from
   the process heap on a thread's first COM use, stores it in `TEB.ReservedForOle`
   (TEB+0x1758 in a 64-bit process, +0xF80 in a 32-bit one; the layouts of the three
   observable fields are in `poly_com::apartment::{BLOCK_X64, BLOCK_X86}`), and never
   frees or clears it. The flags value, the initialisation count and the OLE count are
   the sheet's: 0x81 / 0xC1 / 0x141 / 0x2000C1 / 0x200141 for a first successful call,
   0x1 once everything is undone. The apartment model, `CoGetApartmentType`,
   `CoRegisterMessageFilter` and the answers of activation are all derived from
   it (`poly_com::apartment::ComState`, host-tested). The TLS slot is gone.
2. **Implicit MTA.** A process-wide count of threads that hold the multithreaded
   apartment (`MtaThreads`): a thread with no apartment of its own sees type 1,
   qualifier 1 while it is above zero, and its COM calls work. A thread that ends
   without balancing its initialisations keeps the count up (a thread-exit hook is not
   there yet).
3. **A failing COM call still counts.** `CoCreateInstance`, `CoGetClassObject` and the
   new `CoGetObjectContext` give a thread without an apartment its block (value 0x1)
   although they fail with `CO_E_NOTINITIALIZED`, as the sheet records.
4. **The global interface table** is one static object (`IUnknown` and
   `IGlobalInterfaceTable`, in-process contexts only, no aggregation, `IMarshal`
   refused). Registrations (`poly_com::git::Table`) have a cookie each (never reused,
   never 0, 4 apart from 0x10004), hold one reference of the object, are revoked from
   any apartment, and end with the registering STA's last `CoUninitialize`. In the
   registering apartment a fetch is the object's own `QueryInterface`. In another one
   Windows hands out a proxy; Poly has no marshalling, so the caller gets an object that
   is `IUnknown` and no more, holding no reference of the registered object. That is
   the line to revisit when a program needs a real interface across apartments.
5. **BITS is not registered.** `CLSID_BackgroundCopyManager` is answered like every
   class nothing registers, `REGDB_E_CLASSNOTREG` with a NULL out parameter (the log
   names it: "BITS: not installed here"), because Poly does not have the service.
   On the oracle BITS is there and the creation succeeds, so the two visible tests of
   that entry, which record the oracle, cannot pass; the entry stays not green. Chrome's
   updater takes the refusal and goes on.

## Consequences

- Chrome's S3 window stays up past 160 s in the runs of `impl-com`, with the update
  component enabled; the boot test group `chrome-win` now also checks the window of
  stage 3 (`pixel:200|400|008000` of the green page at about 125 s).
- A program that wants BITS gets a clean refusal today and a real service the day
  someone writes one; nothing in `ole32` has to change for the other classes.
- The 32-bit layout is the sheet's single observation; the 64-bit one is covered by the
  visible tests.
