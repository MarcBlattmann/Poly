# Type-library file format: work in progress (B2b-2, paused)

Status: **PAUSED, not a finished sheet.** Everything here is from black-box observation on the oracle (homeserver2): libraries written
with `CreateTypeLib2` / `ICreateTypeLib2` / `ICreateTypeInfo`, saved, compared byte by byte across one-at-a-time variations, and read back
through `ITypeLib` / `ITypeInfo`. Google's six `updater.exe` libraries (159.0.8140.0) were read only as data (the `TYPELIB` resource bytes) to
check the layout against what the API reports; none of Google's bytes are in the repository. Nothing of Wine, ReactOS or any third-party
write-up was used. Each fact below names its experiment (kit: `poly-generated/spec-win-typelib-format/`, scripts `exp/eN.txt`).
All integers are little endian; `-1` is `0xFFFFFFFF`. Names in this note are the spec agent's own.

## Resource form (e2/e6)

A `TYPELIB` resource of an exe/dll is the bytes of a `.tlb` file as they are (`updater.exe` resources 1..6 start with the same 8 bytes as a `.tlb`).

## Layout (e1, e2, e3, e5; verified on files with 0..33 types)

```
0x00  magic "MSFT" (bytes 4D 53 46 54), 0x04: 0x00010002
0x08  offset of the library GUID in the GUID table (-1: none)
0x0C  LCID used for strings (default 0x2000 when SetLcid is not called)   0x10  the library's LCID (SetLcid; 0 when set to 0)
0x14  flags word: low byte = syskind (win32 -> 0x41, win64 -> 0x43; bit 0x40 always), +0x10 when a help file is set, +0x100 when a help string DLL is set
0x18  version (major | minor << 16)       0x1C  library flags (SetLibFlags; FHASDISKIMAGE 8 is not stored: the loader adds it)
0x20  number of type infos                0x24  help string (offset in the string table, -1 none)
0x28  help string context                 0x2C  help context
0x30  count of unique names               0x34  total characters of those names (ITypeLib2::GetLibStatistics reports both)
0x38  library name (offset in the name table, -1 none)   0x3C  help file (offset in the string table, -1 none)
0x40  library custom data (offset of the first entry in the custom-data GUID table, -1 none)
0x44  four words, 0x20 0x80 -1 N in every file seen so far (N = number of imported types; third word = reference of the last import, e.g. 0xd, when 2+)
[0x54  help string DLL offset, only when flag 0x100 is set]
then   one u32 per type info: its offset in the type-info table
then   the directory: 15 entries of 16 bytes: offset (absolute file offset, -1 absent), length, -1, 0xF
```

Directory slots (e3, e5, real): 0 type-info table (100 bytes per type), 1 import info (12 bytes each), 2 import files, 3 reference table (16 bytes each:
href, impl flags, custom data, next), 4 GUID hash (0x80), 5 GUID table (24 bytes: GUID, href/offset, hash chain), 6 name hash (0x200),
7 name table, 8 string table, 9 type-descriptor table, 10 array-descriptor table, 11 custom-data table, 12 custom-data GUID table (12 bytes: guid, data, next),
13 and 14 unused so far. Data follows the directory in this order; each type's function block comes after all directory sections.
Hash tables (4, 6) are not needed by a reader (scan the tables); their hash function is not yet worked out.

Name table entry: u32 owner (type-info offset, -1 for parameters and the library), u32 next in hash chain, u32 (length | hash << 8), then the name bytes padded to 4 with `W` (0x57).
Names are 8-bit text in the library's code page (a `ü` came back as two bytes; not needed for the updater: all ASCII).
String table entry: u16 length, text, padding to 4 with `W`. GUID-table type entries hold the type-info offset; imported types hold (import-info offset | 1).

Import info (12 bytes): 0x0301_0000 | index of the type in the imported library (IUnknown 0, IDispatch 1 in stdole2), file offset (0 = first import file), offset of the type's GUID in the GUID table.
Import file (e5, 28 bytes for stdole): u32 GUID-table offset of the imported library's GUID, u32 (0 seen; the LCID?), u32 (2 seen; the version?), u16 `(name length << 2) | 3` (0x2F for 11 characters), the file name `stdole2.tlb`, padding with `W` to 4 (meanings of the two middle words are guesses, unverified).

## Type-info record (25 words; e3/e4/e5, matched against the API for 12 kinds/flags variations and the real libraries)

| word | meaning (observed) |
|---|---|
| 0 | bits 0-3 kind (TKIND), bit 4 set on a dual dispatch, bits 11-15 alignment, bits 16+ the type's index |
| 1 | file offset of the type's function block |
| 6 | functions | variables << 16 |
| 11 | GUID offset | 12 | type flags (interface: coclass-only flags are not stored) | 13 | name offset |
| 14 | version (major | minor << 16) | 15 | doc string offset (-1) | 16 | help string context | 17 | help context |
| 19 | vtable size << 16 | number of implemented types | 20 | instance size |
| 21 | alias type descriptor (kind 6), or href of the single implemented type, or offset of the first reference-table entry (coclass) |
| 22 | inherited function count << 16 | inheritance depth (IUnknown 3/1, IDispatch 7/2) |
| 2-5, 7-10, 18, 23, 24 | not decoded yet (0, 0, 3, 0 ... and -1) |

## Function block (e3 `t_if*`)

`u32 size of the records`, then per function: `u32 record size (24 + 12 per parameter)`, return type, function flags, `(size << 16) | vtable offset`, a packed word
(call convention in bits 8-11, invoke kind, function kind), parameter count; then per parameter `(type, name offset, flags)`; after all records three arrays of `n` u32: member ids, name offsets, record offsets.
Simple types are inline: `(0x8000 | vt) << 16 | vt`; others are offsets into the type-descriptor table (entries `(0x4000 | vt?) ...` / `0x7fff001d, href`; not decoded).

## Not done

Packed word of the function record (function kind, invoke kind: `0x4411` seen for a dispatch property get), optional record fields (doc strings, help contexts, entry points, default values, custom data), the
type-descriptor encoding for `PTR`, `USERDEFINED`, `SAFEARRAY`, `CARRAY`, the variable block, custom-data encoding (the MIDL stamps are in every updater library), the dual interface layout, the hash functions,
the remaining type-info words, coclass impl flags, alias types, a reader (Python) checked against `ITypeLib` dumps of all samples and of the six updater libraries, the test programs, sample files, the coverage entry and hand-off.
