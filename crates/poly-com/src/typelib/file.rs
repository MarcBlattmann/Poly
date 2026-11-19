//! Type-library files.
//!
//! [`read`] is the one entry the loader uses: it recognises the file by its first bytes and hands
//! it to the reader of that format. Two formats exist:
//!
//! - **Poly's own** ([`encode`] / [`decode`]): what Poly's `CreateTypeLib2` writes (sheet
//!   `CreateTypeLib2`: "Poly's own CreateTypeLib2 may store libraries in any form it reads back").
//!   Magic `PolyTLB\0`, a version word, then the library: numbers little-endian, strings UTF-8 with
//!   a 32-bit length, GUIDs their 16 bytes in memory order, type descriptions as a tag and their
//!   parts.
//! - **`MSFT`**, the format of libraries MIDL writes (Google's updater has six as resources):
//!   [`super::msft`], which waits for its format sheet (B2b-2).

use super::*;
use alloc::vec;

/// The first bytes of a Poly type-library file.
pub const MAGIC: &[u8; 8] = b"PolyTLB\0";
/// The version of the layout below.
const VERSION: u32 = 2;

/// Why a file is not a library this loader reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Not a type library (an empty file, text, damaged data).
    NotALibrary,
    /// A format Poly cannot read yet (`MSFT` before its sheet).
    Unsupported,
}

/// The library in `bytes`, whatever its format.
pub fn read(bytes: &[u8]) -> Result<Library, ReadError> {
    if bytes.starts_with(MAGIC) {
        return decode(bytes).ok_or(ReadError::NotALibrary);
    }
    if super::msft::recognises(bytes) {
        return super::msft::read(bytes);
    }
    Err(ReadError::NotALibrary)
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn guid(&mut self, g: &Guid) {
        self.u32(g.data1);
        self.u16(g.data2);
        self.u16(g.data3);
        self.0.extend_from_slice(&g.data4);
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn td(&mut self, t: &TypeDesc) {
        match t {
            TypeDesc::Base(vt) => {
                self.u8(0);
                self.u16(*vt);
            }
            TypeDesc::Ptr(i) => {
                self.u8(1);
                self.td(i);
            }
            TypeDesc::SafeArray(i) => {
                self.u8(2);
                self.td(i);
            }
            TypeDesc::UserDefined(h) => {
                self.u8(3);
                self.u32(*h);
            }
            TypeDesc::CArray(i, dims) => {
                self.u8(4);
                self.td(i);
                self.u32(dims.len() as u32);
                for (n, lo) in dims {
                    self.u32(*n);
                    self.i32(*lo);
                }
            }
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    depth: u32,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.b.len() < n {
            return None;
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(self.u32()? as i32)
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn guid(&mut self) -> Option<Guid> {
        Some(Guid { data1: self.u32()?, data2: self.u16()?, data3: self.u16()?, data4: self.take(8)?.try_into().ok()? })
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        String::from_utf8(b.to_vec()).ok()
    }
    /// A count of items each at least `min` bytes long (refuses counts the data cannot hold).
    fn count(&mut self, min: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n.saturating_mul(min) <= self.b.len()).then_some(n)
    }
    fn td(&mut self) -> Option<TypeDesc> {
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let t = match self.u8()? {
            0 => TypeDesc::Base(self.u16()?),
            1 => TypeDesc::Ptr(Box::new(self.td()?)),
            2 => TypeDesc::SafeArray(Box::new(self.td()?)),
            3 => TypeDesc::UserDefined(self.u32()?),
            4 => {
                let inner = self.td()?;
                let n = self.count(8)?;
                let mut dims = Vec::with_capacity(n);
                for _ in 0..n {
                    dims.push((self.u32()?, self.i32()?));
                }
                TypeDesc::CArray(Box::new(inner), dims)
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(t)
    }
}

impl W {
    fn opt_str(&mut self, v: &Option<String>) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                self.str(x);
            }
        }
    }
    fn cust_value(&mut self, v: &CustValue) {
        match v {
            CustValue::I4(x) => {
                self.u8(0);
                self.i32(*x);
            }
            CustValue::Ui4(x) => {
                self.u8(1);
                self.u32(*x);
            }
            CustValue::Bstr(x) => {
                self.u8(2);
                self.str(x);
            }
            CustValue::Other { vt, bits } => {
                self.u8(3);
                self.u16(*vt);
                self.i64(*bits as i64);
            }
        }
    }
    fn opt_cust_value(&mut self, v: &Option<CustValue>) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                self.cust_value(x);
            }
        }
    }
    fn cust(&mut self, c: &CustData) {
        self.u32(c.len() as u32);
        for (g, v) in c {
            self.guid(g);
            self.cust_value(v);
        }
    }
}

impl R<'_> {
    fn opt_str(&mut self) -> Option<Option<String>> {
        Some(match self.u8()? {
            0 => None,
            1 => Some(self.str()?),
            _ => return None,
        })
    }
    fn cust_value(&mut self) -> Option<CustValue> {
        Some(match self.u8()? {
            0 => CustValue::I4(self.i32()?),
            1 => CustValue::Ui4(self.u32()?),
            2 => CustValue::Bstr(self.str()?),
            3 => CustValue::Other { vt: self.u16()?, bits: self.i64()? as u64 },
            _ => return None,
        })
    }
    fn opt_cust_value(&mut self) -> Option<Option<CustValue>> {
        Some(match self.u8()? {
            0 => None,
            1 => Some(self.cust_value()?),
            _ => return None,
        })
    }
    fn cust(&mut self) -> Option<CustData> {
        let mut out = Vec::new();
        for _ in 0..self.count(17)? {
            out.push((self.guid()?, self.cust_value()?));
        }
        Some(out)
    }
}

/// The library as Poly's file format.
pub fn encode(lib: &Library) -> Vec<u8> {
    let mut w = W(vec![]);
    w.0.extend_from_slice(MAGIC);
    w.u32(VERSION);
    w.guid(&lib.guid);
    w.u32(lib.lcid);
    w.i32(lib.syskind);
    w.u16(lib.major);
    w.u16(lib.minor);
    w.u16(lib.flags);
    w.str(&lib.name);
    w.str(&lib.doc);
    w.str(&lib.help_file);
    w.u32(lib.help_context);
    w.u32(lib.help_string_context);
    w.str(&lib.help_string_dll);
    w.cust(&lib.cust);
    match lib.stats {
        None => w.u8(0),
        Some((a, b)) => {
            w.u8(1);
            w.u32(a);
            w.u32(b);
        }
    }
    w.u32(lib.imports.len() as u32);
    for i in &lib.imports {
        w.guid(&i.lib_guid);
        w.u16(i.lib_major);
        w.u16(i.lib_minor);
        w.u32(i.lib_lcid);
        w.guid(&i.type_guid);
        w.str(&i.type_name);
        w.u32(i.type_index.unwrap_or(u32::MAX));
    }
    w.u32(lib.types.len() as u32);
    for t in &lib.types {
        w.str(&t.name);
        w.str(&t.doc);
        w.u32(t.help_context);
        w.u32(t.help_string_context);
        w.guid(&t.guid);
        w.i32(t.kind);
        w.u16(t.flags);
        w.u16(t.major);
        w.u16(t.minor);
        w.u16(t.align);
        w.u16(t.vft);
        w.u32(t.size_instance);
        w.u8(u8::from(t.laid_out));
        w.cust(&t.cust);
        w.u32(t.impl_types.len() as u32);
        for it in &t.impl_types {
            w.u32(it.href);
            w.i32(it.flags);
        }
        w.u32(t.funcs.len() as u32);
        for f in &t.funcs {
            w.i32(f.memid);
            w.i32(f.funckind);
            w.i32(f.invkind);
            w.i32(f.callconv);
            w.u16(f.opt_params as u16);
            w.u16(f.ovft as u16);
            w.td(&f.ret);
            w.u16(f.ret_flags);
            w.u16(f.flags);
            w.str(&f.name);
            w.str(&f.doc);
            w.u32(f.help_context);
            w.u32(f.help_string_context);
            w.u8(u8::from(f.named));
            w.cust(&f.cust);
            match &f.entry {
                None => w.u8(0),
                Some(e) => {
                    w.u8(1);
                    w.str(&e.dll);
                    w.opt_str(&e.name);
                    w.u16(e.ordinal);
                }
            }
            w.u32(f.params.len() as u32);
            for p in &f.params {
                w.td(&p.td);
                w.u16(p.flags);
                w.str(&p.name);
                w.opt_cust_value(&p.default);
                w.cust(&p.cust);
            }
        }
        w.u32(t.vars.len() as u32);
        for v in &t.vars {
            w.i32(v.memid);
            w.str(&v.name);
            w.td(&v.td);
            w.u16(v.flags);
            w.i32(v.varkind);
            w.i64(v.value);
            w.opt_cust_value(&v.const_value);
            w.str(&v.doc);
            w.u32(v.help_context);
            w.u32(v.help_string_context);
            w.cust(&v.cust);
        }
        match &t.alias {
            None => w.u8(0),
            Some(a) => {
                w.u8(1);
                w.td(a);
            }
        }
    }
    w.0
}

/// A library from Poly's file format; `None` when the bytes are not one.
pub fn decode(bytes: &[u8]) -> Option<Library> {
    let mut r = R { b: bytes.strip_prefix(MAGIC.as_slice())?, depth: 0 };
    if r.u32()? != VERSION {
        return None;
    }
    let mut lib = Library::new(SYS_WIN64);
    lib.guid = r.guid()?;
    lib.lcid = r.u32()?;
    lib.syskind = r.i32()?;
    lib.major = r.u16()?;
    lib.minor = r.u16()?;
    lib.flags = r.u16()?;
    lib.name = r.str()?;
    lib.doc = r.str()?;
    lib.help_file = r.str()?;
    lib.help_context = r.u32()?;
    lib.help_string_context = r.u32()?;
    lib.help_string_dll = r.str()?;
    lib.cust = r.cust()?;
    lib.stats = match r.u8()? {
        0 => None,
        1 => Some((r.u32()?, r.u32()?)),
        _ => return None,
    };
    for _ in 0..r.count(44)? {
        lib.imports.push(Import {
            lib_guid: r.guid()?,
            lib_major: r.u16()?,
            lib_minor: r.u16()?,
            lib_lcid: r.u32()?,
            type_guid: r.guid()?,
            type_name: r.str()?,
            type_index: Some(r.u32()?).filter(|&x| x != u32::MAX),
        });
    }
    for _ in 0..r.count(48)? {
        let mut t = TypeEntry {
            name: r.str()?,
            doc: r.str()?,
            help_context: r.u32()?,
            help_string_context: r.u32()?,
            guid: r.guid()?,
            kind: r.i32()?,
            flags: r.u16()?,
            major: r.u16()?,
            minor: r.u16()?,
            align: r.u16()?,
            vft: r.u16()?,
            size_instance: r.u32()?,
            laid_out: r.u8()? != 0,
            cust: r.cust()?,
            ..TypeEntry::default()
        };
        for _ in 0..r.count(8)? {
            t.impl_types.push(ImplType { href: r.u32()?, flags: r.i32()? });
        }
        for _ in 0..r.count(30)? {
            let mut f = Func {
                memid: r.i32()?,
                funckind: r.i32()?,
                invkind: r.i32()?,
                callconv: r.i32()?,
                opt_params: r.u16()? as i16,
                ovft: r.u16()? as i16,
                ret: r.td()?,
                ret_flags: r.u16()?,
                flags: r.u16()?,
                name: r.str()?,
                doc: r.str()?,
                help_context: r.u32()?,
                help_string_context: r.u32()?,
                named: r.u8()? != 0,
                cust: r.cust()?,
                ..Func::default()
            };
            f.entry = match r.u8()? {
                0 => None,
                1 => Some(DllEntry { dll: r.str()?, name: r.opt_str()?, ordinal: r.u16()? }),
                _ => return None,
            };
            for _ in 0..r.count(9)? {
                f.params.push(Param {
                    td: r.td()?,
                    flags: r.u16()?,
                    name: r.str()?,
                    default: r.opt_cust_value()?,
                    cust: r.cust()?,
                });
            }
            t.funcs.push(f);
        }
        for _ in 0..r.count(20)? {
            t.vars.push(Var {
                memid: r.i32()?,
                name: r.str()?,
                td: r.td()?,
                flags: r.u16()?,
                varkind: r.i32()?,
                value: r.i64()?,
                const_value: r.opt_cust_value()?,
                doc: r.str()?,
                help_context: r.u32()?,
                help_string_context: r.u32()?,
                cust: r.cust()?,
            });
        }
        t.alias = match r.u8()? {
            0 => None,
            1 => Some(r.td()?),
            _ => return None,
        };
        lib.types.push(t);
    }
    r.b.is_empty().then_some(lib)
}
