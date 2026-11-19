//! The `MSFT` type-library format, the one MIDL writes (Google's updater carries six such
//! libraries as `TYPELIB` resources). Read from the format sheet
//! `docs/windows/typelib-file-format.md` (track `spec-win-typelib-format`, derived by black-box
//! observation) and nothing else; section numbers below are the sheet's.
//!
//! [`read`] turns the bytes into a [`Library`], doing what section 6 lists. The loader behaviour of
//! section 9 (a dual interface's dispatch form, `IDispatch` as its base) is [`super::view`]'s,
//! which applies to every library; what depends on the file (a 32-bit library's slot sizes, the
//! disk-image flag) is done here.

use super::file::ReadError;
use super::*;

/// The signature an `MSFT` file starts with (`MSFT` in ASCII).
pub const SIGNATURE: &[u8; 4] = b"MSFT";
/// The word after it (section 3).
const VERSION_WORD: u32 = 0x0001_0002;
/// The size of the header without the optional help-string-DLL word (section 3).
const HEADER: usize = 0x54;
/// A type record is 25 words (section 5).
const TYPE_RECORD: usize = 100;
/// An import-info entry is 12 bytes (section 4); an import reference is its offset | 1.
const IMPORT_INFO: usize = 12;

/// Whether `bytes` look like an `MSFT` library.
pub fn recognises(bytes: &[u8]) -> bool {
    bytes.starts_with(SIGNATURE)
}

/// The library in an `MSFT` file.
pub fn read(bytes: &[u8]) -> Result<Library, ReadError> {
    Reader::new(bytes).and_then(|r| r.library()).ok_or(ReadError::NotALibrary)
}

/// Directory slots (section 2).
const S_TYPEINFO: usize = 0;
const S_IMPORT_INFO: usize = 1;
const S_IMPORT_FILES: usize = 2;
const S_REFERENCES: usize = 3;
const S_GUIDS: usize = 5;
const S_NAMES: usize = 7;
const S_STRINGS: usize = 8;
const S_TYPEDESCS: usize = 9;
const S_ARRAYDESCS: usize = 10;
const S_CUSTDATA: usize = 11;
const S_CUSTGUIDS: usize = 12;

/// A block of records (section 5.3): their base, the ids, the name offsets, the record offsets,
/// and where the block ends.
type Block = (usize, Vec<u32>, Vec<u32>, Vec<u32>, usize);

struct Reader<'a> {
    b: &'a [u8],
    /// (offset, length) of each directory slot, `None` when absent.
    dir: [Option<(usize, usize)>; 15],
    /// The offsets of the type records inside the type-info table.
    type_offsets: Vec<usize>,
    /// The pointer size of the file's syskind (4 for win32 files, 8 otherwise).
    file_ptr: usize,
    /// This process's pointer size (what the API reports: section 9).
    ptr: usize,
}

const NONE: u32 = 0xFFFF_FFFF;

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Option<Reader<'a>> {
        if !b.starts_with(SIGNATURE) || u32_at(b, 4)? != VERSION_WORD {
            return None;
        }
        let flags = u32_at(b, 0x14)?;
        let extra = if flags & 0x100 != 0 { 4 } else { 0 };
        let n = u32_at(b, 0x20)? as usize;
        if n > 0x10000 {
            return None;
        }
        let offsets_at = HEADER + extra;
        let mut type_offsets = Vec::with_capacity(n);
        for i in 0..n {
            type_offsets.push(u32_at(b, offsets_at + 4 * i)? as usize);
        }
        let dir_at = offsets_at + 4 * n;
        let mut dir = [None; 15];
        for (k, slot) in dir.iter_mut().enumerate() {
            let off = u32_at(b, dir_at + 16 * k)?;
            let len = u32_at(b, dir_at + 16 * k + 4)? as usize;
            if off != NONE {
                let off = off as usize;
                if off.checked_add(len)? > b.len() {
                    return None;
                }
                *slot = Some((off, len));
            }
        }
        let file_ptr = if flags & 3 == SYS_WIN32 as u32 { 4 } else { 8 };
        Some(Reader { b, dir, type_offsets, file_ptr, ptr: core::mem::size_of::<usize>() })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        u32_at(self.b, at)
    }

    /// The start of a section.
    fn section(&self, slot: usize) -> Option<usize> {
        self.dir[slot].map(|(o, _)| o)
    }

    /// A byte string as characters (8-bit, one character per byte: section 4).
    fn chars(&self, at: usize, n: usize) -> Option<String> {
        let s = self.b.get(at..at.checked_add(n)?)?;
        Some(s.iter().map(|&c| c as char).collect())
    }

    /// A name of the name table (`None` for -1).
    fn name(&self, off: u32) -> Option<Option<String>> {
        if off == NONE {
            return Some(None);
        }
        let at = self.section(S_NAMES)? + off as usize;
        let n = (self.u32(at + 8)? & 0xFF) as usize;
        Some(Some(self.chars(at + 12, n)?))
    }

    /// A string of the string table (`None` for -1).
    fn string(&self, off: u32) -> Option<Option<String>> {
        if off == NONE {
            return Some(None);
        }
        let at = self.section(S_STRINGS)? + off as usize;
        let n = u16::from_le_bytes(self.b.get(at..at + 2)?.try_into().ok()?) as usize;
        Some(Some(self.chars(at + 2, n)?))
    }

    /// A GUID of the GUID table (`None` for -1).
    fn guid(&self, off: u32) -> Option<Option<Guid>> {
        if off == NONE {
            return Some(None);
        }
        let at = self.section(S_GUIDS)? + off as usize;
        let g = self.b.get(at..at + 16)?;
        Some(Some(Guid {
            data1: u32::from_le_bytes(g[0..4].try_into().ok()?),
            data2: u16::from_le_bytes(g[4..6].try_into().ok()?),
            data3: u16::from_le_bytes(g[6..8].try_into().ok()?),
            data4: g[8..16].try_into().ok()?,
        }))
    }

    /// The `HREFTYPE` of a reference (section 4): an import (low bit set) or a local type.
    fn href(&self, r: u32) -> Option<u32> {
        if r & 1 != 0 {
            Some(Href::Import((r as usize & !1) / IMPORT_INFO).encode())
        } else {
            let i = self.type_offsets.iter().position(|&o| o == r as usize).unwrap_or(r as usize / TYPE_RECORD);
            Some(Href::Internal(i).encode())
        }
    }

    /// A type word (section 4): inline (bit 31: the `vt` in the low 16 bits) or a descriptor.
    fn type_word(&self, w: u32, depth: u32) -> Option<TypeDesc> {
        if depth > 32 {
            return None;
        }
        if w & 0x8000_0000 != 0 {
            return Some(TypeDesc::Base((w & 0xFFFF) as u16));
        }
        let at = self.section(S_TYPEDESCS)? + w as usize;
        let vt = (self.u32(at)? & 0xFFFF) as u16;
        let inner = self.u32(at + 4)?;
        Some(match vt {
            VT_PTR => TypeDesc::Ptr(Box::new(self.type_word(inner, depth + 1)?)),
            VT_SAFEARRAY => TypeDesc::SafeArray(Box::new(self.type_word(inner, depth + 1)?)),
            VT_USERDEFINED => TypeDesc::UserDefined(self.href(inner)?),
            VT_CARRAY => {
                let a = self.section(S_ARRAYDESCS)? + inner as usize;
                let elem = self.type_word(self.u32(a)?, depth + 1)?;
                let dims = (self.u32(a + 4)? & 0xFFFF) as usize;
                if dims > 64 {
                    return None;
                }
                let mut d = Vec::with_capacity(dims);
                for k in 0..dims {
                    d.push((self.u32(a + 8 + 8 * k)?, self.u32(a + 12 + 8 * k)? as i32));
                }
                TypeDesc::CArray(Box::new(elem), d)
            }
            other => TypeDesc::Base(other),
        })
    }

    /// A custom-data value (section 4): inline (bit 31) or an entry of the custom-data table.
    fn value(&self, v: u32) -> Option<Option<CustValue>> {
        if v == NONE {
            return Some(None);
        }
        if v & 0x8000_0000 != 0 {
            let vt = ((v >> 26) & 0x1F) as u16;
            let x = v & 0x03FF_FFFF;
            return Some(Some(match vt {
                VT_UI4 | VT_UINT => CustValue::Ui4(x),
                VT_I4 | VT_INT => CustValue::I4(x as i32),
                _ => CustValue::Other { vt, bits: u64::from(x) },
            }));
        }
        let at = self.section(S_CUSTDATA)? + v as usize;
        let vt = u16::from_le_bytes(self.b.get(at..at + 2)?.try_into().ok()?);
        Some(Some(match vt {
            VT_I4 | VT_INT => CustValue::I4(self.u32(at + 2)? as i32),
            VT_UI4 | VT_UINT => CustValue::Ui4(self.u32(at + 2)?),
            VT_BSTR => {
                let n = self.u32(at + 2)? as usize;
                CustValue::Bstr(self.chars(at + 6, n)?)
            }
            _ => CustValue::Other { vt, bits: u64::from(self.u32(at + 2)?) },
        }))
    }

    /// A custom-data chain starting at `head` (section 4, slot 12).
    fn cust(&self, head: u32) -> Option<CustData> {
        let mut out = Vec::new();
        let mut at = head;
        let base = match self.section(S_CUSTGUIDS) {
            Some(b) => b,
            None => return Some(out),
        };
        for _ in 0..4096 {
            if at == NONE {
                break;
            }
            let e = base + at as usize;
            let g = self.guid(self.u32(e)?)?.unwrap_or_default();
            if let Some(v) = self.value(self.u32(e + 4)?)? {
                out.push((g, v));
            }
            at = self.u32(e + 8)?;
        }
        Some(out)
    }

    /// Scale a size or offset of the file's syskind to this process's slots (section 9).
    fn scale(&self, x: usize) -> usize {
        x * self.ptr / self.file_ptr
    }

    fn library(&self) -> Option<Library> {
        let b = self.b;
        let mut lib = Library::new(SYS_WIN64);
        lib.guid = self.guid(u32_at(b, 0x08)?)?.unwrap_or_default();
        lib.lcid = u32_at(b, 0x10)?;
        let flags = u32_at(b, 0x14)?;
        lib.syskind = (flags & 3) as i32;
        let ver = u32_at(b, 0x18)?;
        lib.major = ver as u16;
        lib.minor = (ver >> 16) as u16;
        // The disk-image flag is not stored; the API adds it (section 3).
        lib.flags = u32_at(b, 0x1C)? as u16 | LIBFLAG_FHASDISKIMAGE;
        lib.doc = self.string(u32_at(b, 0x24)?)?.unwrap_or_default();
        lib.help_string_context = u32_at(b, 0x28)?;
        lib.help_context = u32_at(b, 0x2C)?;
        lib.stats = Some((u32_at(b, 0x30)?, u32_at(b, 0x34)?));
        lib.name = self.name(u32_at(b, 0x38)?)?.unwrap_or_default();
        if flags & 0x10 != 0 {
            lib.help_file = self.string(u32_at(b, 0x3C)?)?.unwrap_or_default();
        }
        lib.cust = self.cust(u32_at(b, 0x40)?)?;
        if flags & 0x100 != 0 {
            lib.help_string_dll = self.string(u32_at(b, 0x54)?)?.unwrap_or_default();
        }
        lib.imports = self.imports()?;
        for i in 0..self.type_offsets.len() {
            lib.types.push(self.type_entry(i)?);
        }
        Some(lib)
    }

    /// The import-info table (section 4), one import per entry, in table order.
    fn imports(&self) -> Option<Vec<Import>> {
        let Some((base, len)) = self.dir[S_IMPORT_INFO] else { return Some(Vec::new()) };
        let files = self.section(S_IMPORT_FILES);
        let mut out = Vec::new();
        for k in 0..len / IMPORT_INFO {
            let e = base + k * IMPORT_INFO;
            let w0 = self.u32(e)?;
            let file = self.u32(e + 4)?;
            let w2 = self.u32(e + 8)?;
            let by_guid = (w0 >> 16) & 0xFF == 1;
            let lib_guid = match files {
                Some(f) => self.guid(self.u32(f + file as usize)?)?.unwrap_or_default(),
                None => Guid::default(),
            };
            out.push(Import {
                lib_guid,
                // The file holds no version or LCID of the imported library (section 4): the
                // loader looks it up by its GUID.
                lib_major: 0,
                lib_minor: 0,
                lib_lcid: 0,
                type_guid: if by_guid { self.guid(w2)?.unwrap_or_default() } else { Guid::default() },
                type_name: String::new(),
                type_index: (!by_guid).then_some(w2),
            });
        }
        Some(out)
    }

    fn type_entry(&self, i: usize) -> Option<TypeEntry> {
        let rec = self.section(S_TYPEINFO)? + self.type_offsets[i];
        let w = |k: usize| self.u32(rec + 4 * k);
        let w0 = w(0)?;
        let kind = (w0 & 0xF) as i32;
        let mut t = TypeEntry {
            kind,
            align: ((w0 >> 11) & 0x1F) as u16,
            guid: self.guid(w(11)?)?.unwrap_or_default(),
            flags: w(12)? as u16,
            name: self.name(w(13)?)?.unwrap_or_default(),
            major: w(14)? as u16,
            minor: (w(14)? >> 16) as u16,
            doc: self.string(w(15)?)?.unwrap_or_default(),
            help_string_context: w(16)?,
            help_context: w(17)?,
            cust: self.cust(w(18)?)?,
            vft: self.scale((w(19)? >> 16) as usize) as u16,
            size_instance: w(20)?,
            laid_out: true,
            ..TypeEntry::default()
        };
        if self.file_ptr != self.ptr && matches!(kind, TKIND_INTERFACE | TKIND_DISPATCH | TKIND_COCLASS) {
            // Section 9: a win32 file's pointer-sized types are reported for 8-byte slots.
            t.size_instance = self.scale(t.size_instance as usize) as u32;
            if usize::from(t.align) == self.file_ptr {
                t.align = self.ptr as u16;
            }
        }
        let n_impl = (w(19)? & 0xFFFF) as usize;
        let w21 = w(21)?;
        // A module's word 21 is the offset of its first DLL name in the string table: the DLL a
        // function without its own reports (sheet rule `modules`, "the first DLL name stored";
        // the word as seen in the sample `kinds.tlb`, which the sheet's table does not list).
        let module_dll = if kind == TKIND_MODULE { self.string(w21)?.unwrap_or_default() } else { String::new() };
        if kind == TKIND_ALIAS {
            t.alias = Some(self.type_word(w21, 0)?);
        } else if kind == TKIND_MODULE {
        } else if n_impl == 1 && kind != TKIND_COCLASS {
            if w21 != NONE {
                t.impl_types.push(ImplType { href: self.href(w21)?, flags: 0 });
            }
        } else if n_impl > 0 && w21 != NONE {
            let base = self.section(S_REFERENCES)?;
            let mut at = w21;
            for _ in 0..n_impl.min(4096) {
                if at == NONE {
                    break;
                }
                let e = base + at as usize;
                t.impl_types.push(ImplType { href: self.href(self.u32(e)?)?, flags: self.u32(e + 4)? as i32 });
                at = self.u32(e + 12)?;
            }
        }
        let counts = w(6)?;
        let (nf, nv) = ((counts & 0xFFFF) as usize, (counts >> 16) as usize);
        let mut at = w(1)? as usize;
        if nf > 0 {
            let (funcs, next) = self.funcs(at, nf, kind, &module_dll)?;
            t.funcs = funcs;
            at = next;
        }
        if nv > 0 {
            t.vars = self.vars(at, nv)?;
        }
        Some(t)
    }

    /// A block of `n` records at `at` (section 5.3): the records' base, the ids, the name offsets,
    /// the record offsets, and where the block ends.
    fn block(&self, at: usize, n: usize) -> Option<Block> {
        let total = self.u32(at)? as usize;
        let base = at + 4;
        let arrays = base + total;
        let mut ids = Vec::with_capacity(n);
        let mut names = Vec::with_capacity(n);
        let mut offs = Vec::with_capacity(n);
        for k in 0..n {
            ids.push(self.u32(arrays + 4 * k)?);
            names.push(self.u32(arrays + 4 * (n + k))?);
            offs.push(self.u32(arrays + 4 * (2 * n + k))?);
        }
        Some((base, ids, names, offs, arrays + 12 * n))
    }

    fn funcs(&self, at: usize, n: usize, kind: i32, module_dll: &str) -> Option<(Vec<Func>, usize)> {
        let (base, ids, names, offs, end) = self.block(at, n)?;
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let r = base + offs[k] as usize;
            let size = (self.u32(r)? & 0xFFFF) as usize;
            let ret = self.type_word(self.u32(r + 4)?, 0)?;
            let flags = self.u32(r + 8)? as u16;
            let vtoff = (self.u32(r + 12)? & 0xFFFF) as usize;
            let packed = self.u32(r + 16)?;
            let counts = self.u32(r + 20)?;
            let np = (counts & 0xFFFF) as usize;
            let nopt = (counts >> 16) as i16;
            let n_words = size.checked_sub(24 + 12 * np)? / 4;
            let opt = |j: usize| -> Option<u32> { self.u32(r + 24 + 4 * j) };
            let mut rest = n_words;
            let defaults_at = if packed & 0x1000 != 0 && rest >= np {
                rest -= np;
                Some(rest)
            } else {
                None
            };
            let cust_at = if packed & 0x80 != 0 && rest > np {
                rest -= np + 1;
                Some(rest)
            } else {
                None
            };
            // F0..F5, the trailing ones left out when they equal their defaults.
            let field =
                |j: usize, default: u32| -> Option<u32> { if j < rest.min(6) { opt(j) } else { Some(default) } };
            let help_context = field(0, 0)?;
            let doc = self.string(field(1, NONE)?)?.unwrap_or_default();
            let entry_word = field(2, NONE)?;
            let dll_word = field(4, NONE)?;
            let help_string_context = field(5, 0)?;
            let params_at = r + 24 + 4 * n_words;
            let mut params = Vec::with_capacity(np);
            for p in 0..np {
                let q = params_at + 12 * p;
                let mut prm = Param {
                    td: self.type_word(self.u32(q)?, 0)?,
                    name: self.name(self.u32(q + 4)?)?.unwrap_or_default(),
                    flags: self.u32(q + 8)? as u16,
                    ..Param::default()
                };
                if let Some(d) = defaults_at {
                    prm.default = self.value(opt(d + p)?)?;
                }
                if let Some(c) = cust_at {
                    prm.cust = self.cust(opt(c + 1 + p)?)?;
                }
                params.push(prm);
            }
            let entry = (kind == TKIND_MODULE).then(|| -> Option<DllEntry> {
                let dll = self.string(dll_word)?.unwrap_or_else(|| String::from(module_dll));
                Some(if packed & 0x2000 != 0 {
                    DllEntry { dll, name: None, ordinal: entry_word as u16 }
                } else {
                    DllEntry { dll, name: self.string(entry_word)?, ordinal: 0 }
                })
            });
            let entry = match entry {
                Some(e) => Some(e?),
                None => None,
            };
            out.push(Func {
                memid: ids[k] as i32,
                funckind: (packed & 7) as i32,
                invkind: ((packed >> 3) & 0xF) as i32,
                callconv: ((packed >> 8) & 0xF) as i32,
                params,
                opt_params: nopt,
                ovft: self.scale(vtoff) as i16,
                ret,
                ret_flags: 0,
                flags,
                name: self.name(names[k])?.unwrap_or_default(),
                doc,
                help_context,
                named: true,
                help_string_context,
                cust: match cust_at {
                    Some(c) => self.cust(opt(c)?)?,
                    None => Vec::new(),
                },
                entry,
            });
        }
        Some((out, end))
    }

    fn vars(&self, at: usize, n: usize) -> Option<Vec<Var>> {
        let (base, ids, names, offs, _) = self.block(at, n)?;
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let r = base + offs[k] as usize;
            let size = (self.u32(r)? & 0xFFFF) as usize;
            let td = self.type_word(self.u32(r + 4)?, 0)?;
            let flags = self.u32(r + 8)? as u16;
            let varkind = (self.u32(r + 12)? & 0xFFFF) as i32;
            let raw = self.u32(r + 16)?;
            let n_words = size.saturating_sub(20) / 4;
            let opt = |j: usize, default: u32| -> Option<u32> {
                if j < n_words { self.u32(r + 20 + 4 * j) } else { Some(default) }
            };
            let (value, const_value) = if varkind == 2 {
                let v = self.value(raw)?;
                let x = match &v {
                    Some(CustValue::I4(x)) => i64::from(*x),
                    Some(CustValue::Ui4(x)) => i64::from(*x),
                    _ => 0,
                };
                (x, v)
            } else {
                (i64::from(raw), None)
            };
            out.push(Var {
                memid: ids[k] as i32,
                name: self.name(names[k])?.unwrap_or_default(),
                td,
                flags,
                varkind,
                value,
                const_value,
                help_context: opt(0, 0)?,
                doc: self.string(opt(1, NONE)?)?.unwrap_or_default(),
                help_string_context: opt(2, 0)?,
                cust: Vec::new(),
            });
        }
        Some(out)
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typelib::view::{Form, view};
    use alloc::vec;

    const SAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../specs/windows/oleaut32.dll/TypeLibFile.samples/");

    fn sample(name: &str) -> Option<Library> {
        let bytes = std::fs::read(alloc::format!("{SAMPLES}{name}")).ok()?;
        Some(read(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}")))
    }

    extern crate std;

    #[test]
    fn library_attributes() {
        let Some(l) = sample("lib_attrs.tlb") else { return };
        assert_eq!((l.lcid, l.syskind, l.major, l.minor, l.flags), (1033, 3, 2, 3, 0xb));
        assert_eq!(l.name, "AttrLib");
        assert_eq!(l.doc, "Library with all attributes");
        assert_eq!((l.help_context, l.help_string_context), (4660, 119));
        assert_eq!(l.help_file, "attrs.hlp");
        assert_eq!(l.help_string_dll, "attrs_strings.dll");
        assert_eq!(l.stats, Some((2, 11)));
        let c: Vec<String> = l.cust.iter().map(|(g, v)| alloc::format!("{:x}={v:?}", g.data1)).collect();
        assert_eq!(
            c,
            ["b2b0f0c4=I4(5)", "b2b0f0c3=Bstr(\"Custom text value\")", "b2b0f0c2=Ui4(4000000000)", "b2b0f0c1=I4(-7)"]
        );
        let t = &l.types[0];
        assert_eq!((t.name.as_str(), t.kind, t.flags, t.major, t.minor, t.align), ("IOne", 3, 0x100, 1, 4, 4));
        assert_eq!((t.help_context, t.help_string_context, t.doc.as_str()), (33, 34, "First interface"));
        let v = view(&l, 0, Form::Stored).unwrap();
        assert_eq!((v.cb_size_instance, v.cb_size_vft), (8, 24));
        assert_eq!(l.imports[0].type_guid, stdole::IID_IUNKNOWN);
    }

    #[test]
    fn win32_and_empty() {
        let Some(l) = sample("lib_win32.tlb") else { return };
        assert_eq!((l.syskind, l.lcid, l.flags), (1, 0, 8));
        let v = view(&l, 0, Form::Stored).unwrap();
        assert_eq!((v.cb_size_instance, v.cb_size_vft, v.align), (8, 32, 8));
        assert_eq!((v.funcs[0].ovft, v.funcs[0].memid, v.funcs[0].name.as_str()), (24, 0x6001_0000, "Ping"));
        let Some(e) = sample("lib_empty.tlb") else { return };
        assert_eq!((e.guid, e.lcid, e.types.len(), e.stats), (Guid::default(), 0x2000, 0, Some((0, 0))));
    }

    #[test]
    fn kinds() {
        let Some(l) = sample("kinds.tlb") else { return };
        let names: Vec<&str> = l.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names[..4], ["Color", "Pair", "Unio", "NativeFns"]);
        let color = &l.types[0];
        let consts: Vec<(i32, i64)> = color.vars.iter().map(|v| (v.memid, v.value)).collect();
        assert_eq!(consts, [(0x64, 1), (0x65, 2), (0x66, -5), (0x67, 100_000_000)]);
        assert_eq!((color.vars[1].doc.as_str(), color.vars[1].help_context), ("Green one", 9));
        let pair = view(&l, 1, Form::Stored).unwrap();
        assert_eq!(pair.cb_size_instance, 44);
        assert_eq!(pair.vars[3].td, TypeDesc::CArray(Box::new(TypeDesc::Base(VT_I4)), vec![(4, 1)]));
        let fns = &l.types[3];
        let e: Vec<Option<DllEntry>> = fns.funcs.iter().map(|f| f.entry.clone()).collect();
        assert_eq!(e[0], Some(DllEntry { dll: "kernel32".into(), name: Some("Beep".into()), ordinal: 0 }));
        assert_eq!(e[2], Some(DllEntry { dll: "user32".into(), name: None, ordinal: 7 }));
        assert_eq!(e[3], Some(DllEntry { dll: "kernel32".into(), name: None, ordinal: 0 }));
        let class = &l.types[10];
        assert_eq!(class.impl_types.iter().map(|i| i.flags).collect::<Vec<_>>(), [1, 2]);
        let disp = view(&l, 9, Form::Stored).unwrap();
        assert_eq!((disp.funcs.len(), disp.impl_types.len(), disp.funcs[2].params[1].flags), (3, 1, 0x11));
    }

    #[test]
    fn dual_projection() {
        let Some(l) = sample("dual.tlb") else { return };
        let d = view(&l, 1, Form::Stored).unwrap();
        assert_eq!((d.kind, d.flags, d.funcs.len(), d.cb_size_vft), (TKIND_DISPATCH, 0x1040, 13, 56));
        let f: Vec<(i32, i16, i32)> = d.funcs.iter().map(|f| (f.memid, f.ovft, f.invkind)).collect();
        assert_eq!(f[7], (0x6002_0000, 56, 2));
        assert_eq!(f[10], (0x6003_0000, 80, 2));
        assert_eq!(f[11], (0x6003_0000, 88, 4));
        assert_eq!(d.funcs[0].ret, TypeDesc::Base(VT_VOID));
        assert_eq!(d.funcs[9].ret, TypeDesc::Ptr(Box::new(TypeDesc::Base(VT_DISPATCH))));
        assert_eq!(d.funcs[11].name, "extra");
        assert_eq!(d.impl_types[0].href, Href::Stdole(stdole::T_IDISPATCH).encode());
        let alias = &l.types[2];
        assert_eq!((alias.kind, alias.alias.clone()), (TKIND_ALIAS, Some(TypeDesc::Base(VT_UI8))));
        // Every sample reads.
        for s in ["funcs.tlb", "lib_many.tlb", "updater_like.tlb"] {
            let lib = sample(s).unwrap();
            for t in 0..lib.types.len() {
                assert!(view(&lib, t, Form::Stored).is_some(), "{s} {t}");
            }
        }
    }
}
