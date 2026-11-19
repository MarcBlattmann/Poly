# The type-library file (`.tlb`, and the `TYPELIB` resource of an exe or dll): format sheet

Track `spec-win-typelib-format` (spec agent, 2026-11-17 to 2026-11-19), batch B2b-2 of [chrome-installer-plan.md](chrome-installer-plan.md); the plan is in
[com-typelib-plan.md](com-typelib-plan.md), the updater's libraries as the API reports them in [com-typelib-updater-types.md](com-typelib-updater-types.md).
For the implementer of `LoadTypeLib` / `LoadTypeLibEx` (`impl-chrome-installer-com-b2b`): this is what Poly has to **read**. Writing the format is not needed
(Poly's own `CreateTypeLib2` may store a library in any form it reads back).

## How this was found (clean room)

Black-box observation on the oracle (homeserver2) only. A small program of ours (`tlgen`, a script-driven caller of `CreateTypeLib2`, `ICreateTypeLib2`,
`ICreateTypeInfo(2)` from Microsoft Learn's oaidl.h) wrote about 150 libraries; each was saved, the bytes were compared across controlled variations (one more
function, one more parameter, another `VT_*`, another flag, ...), and every hypothesis was checked by reading the same file back through
`ITypeLib` / `ITypeInfo` / `ITypeLib2` / `ITypeInfo2` and comparing field by field. The six libraries inside the updater (`updater.exe` 159.0.8140.0, resources
`TYPELIB` 1 to 6) were read as **data** (the resource bytes) and compared the same way: a reader written from this sheet reports **0 differences** from the API for resources
2 and 5, and for 1, 3, 4 and 6 only the loader behaviour listed in section 9 (about 900 function records). No Microsoft or Google binary was disassembled or decompiled;
no Wine, ReactOS or third-party description of the format was read. The names of fields below are the spec agent's own. Experiments are named (`e4`, ...) in the
observation kit `poly-generated/spec-win-typelib-format/`; the sample files in `specs/windows/oleaut32.dll/TypeLibFile.samples/` are made by our own program.

Integers are little endian; `-1` is `0xFFFFFFFF`; "offset" is a byte offset from the start of the file unless a table is named (then it is from the start of that table).
"Padding" bytes are `W` (0x57) in every table that pads strings.

## 1. What the updater's libraries use (the scope)

Library: LCID words 0x409/0, syskind 3 (64-bit), version 1.0, doc string, **custom data on the library** (MIDL's three stamps: two `ui4`, one `bstr`), imports of
`stdole2.tlb` (IUnknown, IDispatch). Types: coclass (kind 5, flags 2, doc string, one implemented type `IUnknown` with flag 1), custom interfaces (kind 3, flag 0x100,
one implemented type `IUnknown`, doc string), duals (stored as kind 4 with flags 0x1140, implemented type `IDispatch` or a local dual), one alias (kind 6, `ULONG_PTR` = `ui8`,
with **custom data on the type**). Functions: ids `0x6001nnnn`/`0x6002nnnn`/`0x6003nnnn`, invoke kinds function/get/put, `stdcall`, return `HRESULT`, parameter types `i4 ui4 ui2 uint int i8
ptr(i4) ptr(i8) ptr(bstr) ptr(bool) bstr bool lpwstr variant ptr(variant) date dispatch ptr(ptr(...))`, pointers to types of the same library, user types (the alias), flags in, out, in+out, retval (`out+retval`).
Not used by the updater, observed and described anyway: enums, records, unions, modules (entry points), variables, default values, function/parameter custom data, help strings of members, carrays, safearrays, win32 (syskind 1) files.

## 2. Overall layout

```
0x00            header (0x54 bytes)
0x54            [help string DLL offset, 4 bytes, only when flag 0x100 of word 0x14 is set]
                one u32 per type: the offset of its record inside the type-info table
                the directory: 15 entries of 16 bytes
                the sections the directory points to, in slot order, each 4-aligned, without gaps
                the per-type data blocks (functions, variables), in type order
```

The directory entry is `offset (absolute, -1 when the section is absent), length in bytes, -1, 0xF`; the last two words were the same in every file. Slots:

| slot | section | entry size |
|---|---|---|
| 0 | type-info table | 100 bytes per type |
| 1 | import info | 12 |
| 2 | import files | variable |
| 3 | reference table | 16 |
| 4 | GUID hash table | 0x80 bytes (32 slots of u32) |
| 5 | GUID table | 24 |
| 6 | name hash table | 0x200 bytes (128 slots) |
| 7 | name table | variable (12 + the name padded to 4) |
| 8 | string table | variable |
| 9 | type-descriptor table | 8 |
| 10 | array-descriptor table | variable |
| 11 | custom-data table | variable |
| 12 | custom-data GUID table | 12 |
| 13, 14 | not used (always absent) | |

A section is present only when it has content (a library with no import has no slot 1/2, no custom data none of 11/12, and so on). The hash tables (4, 6) exist in every file (all slots -1 when empty);
**a reader does not need them** (look up by scanning the tables); their hash function is not described here.

## 3. Header

| offset | meaning (observed as) | experiment |
|---|---|---|
| 0x00 | magic `MSFT` (bytes 4D 53 46 54) | all |
| 0x04 | 0x00010002 | all |
| 0x08 | offset of the library GUID inside the GUID table, -1 none | e2 `h_guid` (0 with a GUID, -1 without) |
| 0x0C | LCID used for strings: SetLcid value; 0x2000 when SetLcid was never called; **0x409 when SetLcid(0)** | e2 `h_lcid*` |
| 0x10 | the library's LCID (what `GetLibAttr` returns): SetLcid value, 0x2000 by default | e2 |
| 0x14 | low 2 bits syskind (win32 1 -> 0x41, win64 3 -> 0x43; bit 0x40 always set); +0x10 when a help file is set; +0x100 when a help string DLL is set | e2 `h_win32`, `h_helpfile`, `h_hsdll` |
| 0x18 | version: major in the low 16 bits, minor in the high 16 | e2 `h_ver*` (65535.65534 -> 0xFFFEFFFF) |
| 0x1C | library flags as set (`SetLibFlags`), but bit 8 (has disk image) is not stored; the API adds 8 | e2 `h_flags*` |
| 0x20 | number of type infos | e3 |
| 0x24 | help string: offset in the string table, -1 none (set together with the doc string) | e2 `h_doc` |
| 0x28 | help string context | e2 `h_hsctx` |
| 0x2C | help context | e2 `h_helpctx` |
| 0x30 | number of distinct names in the name table | `ITypeLib2::GetLibStatistics` agrees |
| 0x34 | total characters of those names | same |
| 0x38 | library name: offset in the name table, -1 none | e2 `h_name` |
| 0x3C | help file: offset in the string table, -1 none | e2 `h_helpfile` |
| 0x40 | custom data of the library: offset of the first entry of the custom-data GUID table (a chain), -1 none | e2 `h_cust*`, updater |
| 0x44 | 0x20 = number of slots of the GUID hash table | all |
| 0x48 | 0x80 = number of slots of the name hash table | all |
| 0x4C | import reference (see 5.4) of the imported `IDispatch`; -1 when the library does not import it | e5, e8, e10 |
| 0x50 | number of entries of the import-info table | e5, e8 |
| 0x54 | help string DLL offset (string table), only when flag 0x100 is set; the type offsets and the directory shift by 4 | e2 `h_hsdll` |

The API reports the help file as `<directory of the library file>\<name>` followed by a NUL character; the file stores only `<name>`.

## 4. Tables

**GUID table** (slot 5), 24 bytes: the 16 GUID bytes (the ordinary in-memory layout), a u32 *reference* (the offset of the owning type record in the type-info table; for the library GUID 0xFFFFFFFE; for an imported library GUID 2; for an imported type its import reference, see 5.4)
and a u32 next entry in the hash chain (an offset inside the GUID table, -1 end). Type GUIDs, the library GUID and the GUIDs of imported libraries and types share the table. e3, e5.

**Name table** (slot 7): per name a u32 owner (offset of the type record the name belongs to, -1 for the library name and parameter names), a u32 next in the hash chain (-1 end), a u32 `length | hash << 8` (length in the low byte),
then the name's bytes (8-bit characters, the code page of the library), padded with `W` to a multiple of 4. A name used twice is stored once. A reader reads a name at `section + offset`. e3 (`IFoo`, `Bar`, `a`).
Non-ASCII characters were seen stored as the UTF-8 bytes of the character and reported as two characters by the API; the updater uses only ASCII names. e2 `h_name_u`.

**String table** (slot 8): per string a u16 length in bytes, the bytes, padding to 4 with `W`. Doc strings, help file, help string DLL, module entry names and DLL names live here; an offset is relative to the table. e2 `h_doc`, e11.

**Import files** (slot 2), observed for `stdole2.tlb` only (28 bytes): u32 offset of the imported library's GUID in the GUID table, u32 0, u32 2, u16 `(name length << 2) | 3`, the file name `stdole2.tlb`, padding. (The meaning of the two middle words is not known; a reader needs only the GUID.)

**Import info** (slot 1), 12 bytes per imported type: `kind << 24 | flag << 16 | n` where `kind` is the imported type's TKIND (3 interface, 1 record), `flag` is 1 when the type is identified by its GUID, 0 when it is identified by its index in the imported library, and `n` counts the import entries (0, 1, 2, ...);
the offset of the import file (0); then the offset of the type's GUID in the GUID table (flag 1) or the type's index in the imported library (flag 0: `GUID` 0, `DISPPARAMS` 1, `EXCEPINFO` 2 in `stdole2.tlb`). The writer adds one entry per `AddRefTypeInfo` call, also for a type it already imported (e8: three entries for `GUID`).
**Import reference** = offset of the entry inside the import-info table | 1; it is what the GUID table, type descriptors and type records hold for an imported type. A local type's reference is the offset of its record in the type-info table (a multiple of 100, low bit 0).

**Reference table** (slot 3), 16 bytes per implemented type of a coclass (and of an interface with more than one): reference of the type, implemented-type flags (`IMPLTYPEFLAG_*`), custom-data chain head (-1), offset of the next entry (-1 last). e5 `c_two`.

**Type-descriptor table** (slot 9), 8 bytes: word 0 = `vt | x << 16` (the high half is derived: 0x4000|base vt for a pointer to a simple type, 0x2000|base vt for a safearray of one, 0x7FFF or 0x7FFE otherwise; a reader ignores it), word 1 = the inner type for `VT_PTR` (26) and `VT_SAFEARRAY` (27),
the reference for `VT_USERDEFINED` (29), the offset of an array descriptor for `VT_CARRAY` (28). An **inner/outer type word** (in word 1, and in return, parameter, variable and alias fields) is either *inline*: bit 31 set, the `vt` in the low 16 bits (the high half repeats it as `0x8000 | base vt`, with `VT_INT` and `VT_UINT` as base `I4`/`UI4`), or an offset into this table. e8, e10.

**Array-descriptor table** (slot 10): the element type word, `dims | (8 * dims) << 16`, then `(element count, lower bound)` per dimension. e8 `o_t_ca`.

**Custom-data GUID table** (slot 12), 12 bytes: offset of the GUID in the GUID table, the value, the offset of the next entry (-1 last); the chain starts at the owner's head word (library 0x40, type record word 18, function record, reference table). The **value** is *inline* when bit 31 is set: `0x80000000 | vt << 26 | value` (a 26-bit value; `i4=1` is 0x8C000001),
otherwise an offset into the custom-data table (slot 11): `u16 vt` then 4 bytes (`I4`, `UI4`) or `u32 length` + the characters (`BSTR`), padded to 4. Seen: i4, ui4, bstr. e2 `h_cust*`, e8, updater.

## 5. Type-info record (slot 0, 25 words)

| word | meaning | experiment |
|---|---|---|
| 0 | bits 0-3 TKIND (0 enum, 1 record, 2 module, 3 interface, 4 dispatch, 5 coclass, 6 alias, 7 union); bit 4 set on a **dual** (a type created as interface with `TYPEFLAG_FDUAL` is stored as kind 4 with this bit); bits 5-10 derived; **bits 11-15 the alignment** (the API's `cbAlignment`: 1, 2, 4, 8); bits 16-31 the type's index | e4 `a_*`, `k_*`, `f_40` |
| 1 | offset of the type's data block (5.3); equal to the end of the sections when the type has none | e3 |
| 2, 3, 5, 7-10, 23 | 0 | all |
| 4 | 3 | all |
| 6 | number of functions (low 16) \| number of variables << 16 | e3, e10 |
| 11 | offset of the type's GUID in the GUID table (-1 none) | e3 |
| 12 | type flags (`SetTypeFlags`); the coclass-only flags 1, 2, 4, 8, 0x20, 0x400 are kept only for coclasses | e4 `f_*` |
| 13 | name: offset in the name table | e3 |
| 14 | version, major low / minor high | e4 `v_*` |
| 15 | doc string: offset in the string table, -1 none | e4 `d_*` |
| 16 | help string context | e4 `c_hs` |
| 17 | help context | e4 `c_h` |
| 18 | custom data chain head (type), -1 none | updater alias, e2 |
| 19 | `cbSizeVft << 16 | number of implemented types`; the vtable size counts the inherited slots (IUnknown 3, IDispatch 7, in pointer-size slots of the file's syskind) | e5 |
| 20 | `cbSizeInstance` (the API reports it): interface/coclass/dispatch pointer size, enum 4, record/union the data size, module 2 | e4 `k_*` |
| 21 | kind 6 (alias): the aliased type word; else: the **reference** of the implemented type when there is exactly one and it is not a coclass, the **offset of the first reference-table entry** otherwise (coclass, or more than one); -1 none | e5, e10 |
| 22 | inherited function count << 16 \| inheritance depth: `IUnknown` base 3/1, `IDispatch` base 7/2, a local base: its total/depth+1 (0x170003 = 23 inherited, depth 3 in the updater) | e5, updater |
| 24 | -1 | all |

A pure dispinterface (kind 4, no dual bit, `impl IDispatch`) stores -1 in word 21 although it has one implemented type (the API reports `IDispatch`). e7 `fn_d`.

### 5.3 The per-type block

After all sections (the blocks of the types follow each other in type order). **Functions**, when word 6 has a function count n: `u32` total size of the n records, the n records, then three arrays of n u32: **member ids**, **name offsets** (name table; -1 none; a put with the same id as its get repeats the get's name), **record offsets** (relative to the first record).
**Variables** (word 6 high half, enums/records/unions/dispinterfaces): the same shape directly after the function part: `u32` total size, the records, ids, names, offsets. (A type with both was not verified.)

**Function record** (size 24 + 12 per parameter + 4 per optional word):

```
+0   u32  size (low 16 bits) | function index << 16
+4   u32  return type word
+8   u32  function flags (FUNCFLAGS, as set)
+12  u32  vtable offset in bytes (low 16: the slot index * pointer size of the file's syskind)  |  informational size << 16 (0x34 + 0x10 per parameter + 8 per non-inline type; ignore)
+16  u32  packed: bits 0-2 function kind (1 pure virtual, 3 static, 4 dispatch); bits 3-6 invoke kind (1 function, 2 get, 4 put, 8 putref); bit 7 has custom data;
          bits 8-11 calling convention (4 stdcall, 1 cdecl, 2 pascal, 0 fastcall); bit 12 has default values; bit 13 the module entry is an ordinal;
          bit 14 some parameter is [retval] or [lcid]; bits 16-31 the function index again
+20  u32  parameter count (low 16) | optional-parameter count << 16
+24  optional words (below), then the parameters: 3 words each: type word, name offset (-1 none), parameter flags (PARAMFLAG_*, as set; 0x20 marks a default value)
```

**Optional words** (their count is (size - 24 - 12 * parameters) / 4; a trailing field equal to its default is left out; the fields in this order):
F0 help context (0), F1 doc string offset (-1), F2 for a module function the entry-point name offset, or the ordinal when bit 13 is set (-1), F3 -1, F4 the DLL name offset for a module function (0 when absent: the first string), F5 help string context (0),
then, when bit 7 is set: F6 the function's custom-data chain head (-1) and one chain head per parameter (-1); then, when bit 12 is set, one value per parameter (the inline form of 4, or an offset into the custom-data table; -1 none).
So a function with only a doc string has 2 optional words (`0`, offset); with only default values has n; with custom data has at least 6 + 1 + n. e8, e11, e12.

**Variable record**: `size | index << 16`, type word, variable flags, `size-ish << 16 | VARKIND` (0 per instance, 1 static, 2 constant, 3 dispatch), the value: for a constant the inline custom-data form or an offset into the custom-data table (`i4=-5` is a table entry, `i4=1` inline), otherwise the instance offset; then optional words: help context, doc string. e10 `v_enum`, e11 `m_c`.

## 6. Reading algorithm (what `LoadTypeLibEx` has to do)

1. Check `MSFT` and 0x00010002. Read the header, the type offsets (after the optional word) and the directory (slot k at `0x54 + [4] + 4 * n + 16 * k`).
2. Library: GUID, LCID (word 0x10), version, flags (| 8), name, doc string (= the help string), help file name, help contexts, custom data chain.
3. For each type: record (word 0 kind, GUID, flags, name, version, doc, contexts, alignment), implemented types (word 21/ reference table), function block, variable block.
4. A type word: inline (bit 31) or a descriptor chain; a reference resolves to a local type (offset into the type-info table: divide by 100 for the index) or an import (look the GUID or the index up in `stdole2.tlb`, which the loader finds through `LoadRegTypeLib`).
5. `GetTypeInfoOfGuid`: scan the GUID table; `FindName` / `IsName`: scan the name table ignoring case; both orders agree with the file's hash tables for every sample (`layout_many`: 70 types, upper-case lookups).

## 7. Resource form

A `TYPELIB` resource is the file's bytes unchanged (`updater.exe`'s six resources start with the same 8 bytes as a `.tlb`; a DLL image written by a test with the sample as resource 3 loads through `file.dll\3`; see `LoadTypeLibEx` for the syntax). Test `layout_resource`.

## 8. Writing side facts (for Poly's own `CreateTypeLib2`, optional)

`ICreateTypeInfo::AddFuncDesc` accepts only function kind pure virtual for interfaces and dispatch for dispinterfaces (anything else: `0x800288BD`), `SetTypeFlags(FDUAL)` on a dispatch type is refused (use an interface type), a put and a get with one member id share the name
(`SetFuncAndParamNames` for the put with a parameter list fails), interfaces refuse `SetImplTypeFlags`. Not needed for reading.

## 9. What the API reports that is not in the file (loader behaviour a reader must reproduce)

| API shows | file holds | observed |
|---|---|---|
| a **dual** (kind 4 with dual bit) has the 7 `IUnknown`/`IDispatch` methods first (ids 0x60000000.. and 0x60010000.., flags `FRESTRICTED`, `QueryInterface` returns void with `riid`/`ppvObj`), then its own; a dual that derives from a **local dual** has the base's functions (all levels) before its own, and reports `IDispatch` as its single implemented type | the dual's own pure-virtual records, `impl` = IDispatch (reference with bit 0) or the local base | updater 1, 3, 4, 6; `dual` sample |
| those own functions as **dispatch functions** (fk 4): `[retval]` parameter removed and its pointee becomes the return type; a function without retval returns `void` instead of `HRESULT`; vtable offsets continue after the inherited slots | return `HRESULT`, `[out, retval] ptr(T)` last | same |
| type flags 0x1040 for a dual | 0x1140 | same |
| `cbSizeVft` 56 for a dual | 56 + 8 per own slot | same |
| the alias's GUID all zero | no GUID (-1) | updater 3, 6 |
| a **syskind 1** file loaded by a 64-bit process: vtable offsets, `cbSizeVft`, `cbSizeInstance` and alignment scaled to 8-byte slots (offset 12 -> 24, size 4 -> 8, alignment 4 -> 8) | 4-byte slots | `lib_win32` |
| the API's `cbSizeVft` for a plain interface = 8 * (inherited + own slots) | the same total | updater 2, 5 |
| `GetDocumentation` of the library/types: name, doc string; `GetDocumentation2`: the doc string is also the help string (`hs`), the help string context and DLL | | e2 |

## 10. Not covered

The hash functions; word 24 of the type record; the meaning of the middle words of an import file entry and of `0x0C` vs `0x10`'s role in string conversion beyond the values listed; custom data on implemented types; types holding both functions and variables;
non-ASCII names and other code pages; encoded 32-bit-only libraries other than `lib_win32`; the `idl` flags of `TYPEATTR` (always 0 in the samples); `SetMops`, `SetSchema` (stored nowhere visible). Libraries with more than 70 types or 4 GiB-scale offsets were not made.
