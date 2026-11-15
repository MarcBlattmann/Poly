//! Run-time type information as Visual C++ lays it out on x64 (`vcruntime140.dll`'s
//! `__RTtypeid`, `__RTCastToVoid` and `__RTDynamicCast`; the spec sheets of those names, their
//! `rtti_locator_layout` records), decided on memory read through [`Memory`].
//!
//! ```text
//! vptr[-1]                 -> Complete Object Locator
//! CompleteObjectLocator    u32 signature (1), u32 offset (of this vptr's subobject in the
//!                          complete object), u32 cdOffset, rva TypeDescriptor (of the complete
//!                          type), rva ClassHierarchyDescriptor, rva self (image base = COL - self)
//! ClassHierarchyDescriptor u32 signature (0), u32 attributes (0x1 multiple, 0x2 virtual,
//!                          0x4 ambiguous), u32 numBaseClasses, rva BaseClassArray
//! BaseClassArray           numBaseClasses rvas: the class itself, then every base subobject,
//!                          depth first
//! BaseClassDescriptor      rva TypeDescriptor, u32 numContainedBases, i32 mdisp, i32 pdisp,
//!                          i32 vdisp, u32 attributes, rva ClassHierarchyDescriptor
//! ```
//!
//! `dynamic_cast` (C++ [expr.dynamic.cast] p8 and the sheet's `rules`): the search starts from
//! the complete object. The source subobject is the base entry of the source type at the
//! pointer's address. A target subobject that contains it (a downcast) is taken when the source
//! is reachable by a public path; otherwise a target that occurs once in the complete object,
//! publicly, is taken when the source is a public base of the complete object (a cast
//! sideways, also from a base type that occurs more than once, as observed).

use crate::throw::{CatchableType, adjust, same_name};
use crate::{Bad, MAX_ENTRIES, Memory, at, offset, read_i32, read_u32, read_u64};
use alloc::vec;
use alloc::vec::Vec;

/// Base class descriptor attributes: the base is not visible (not a public path), reached
/// through a private or protected path, a private base.
pub const BCD_NOT_VISIBLE: u32 = 0x01;
pub const BCD_AMBIGUOUS: u32 = 0x02;
pub const BCD_PRIVATE_OR_PROTECTED: u32 = 0x04;
pub const BCD_PRIVATE: u32 = 0x08;
/// The bits that make a base unusable as a path for a cast.
pub const BCD_NOT_PUBLIC: u32 = BCD_NOT_VISIBLE | BCD_PRIVATE_OR_PROTECTED | BCD_PRIVATE;

/// A Complete Object Locator, with where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locator {
    pub address: u64,
    pub signature: u32,
    pub offset: u32,
    pub cd_offset: u32,
    pub type_descriptor: u32,
    pub class_descriptor: u32,
    pub self_rva: u32,
}

impl Locator {
    pub fn read(mem: &dyn Memory, address: u64) -> Result<Locator, Bad> {
        let f = |i: u64| read_u32(mem, address.checked_add(4 * i).ok_or(Bad::Overflow)?);
        Ok(Locator {
            address,
            signature: f(0)?,
            offset: f(1)?,
            cd_offset: f(2)?,
            type_descriptor: f(3)?,
            class_descriptor: f(4)?,
            self_rva: f(5)?,
        })
    }

    /// The base of the image that holds it (x64: the locator's own address minus its rva).
    pub fn image_base(&self) -> u64 {
        self.address.wrapping_sub(u64::from(self.self_rva))
    }
}

/// One base class entry of a hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Base {
    /// The type descriptor (absolute).
    pub type_descriptor: u64,
    pub contained: u32,
    pub mdisp: i32,
    pub pdisp: i32,
    pub vdisp: i32,
    pub attributes: u32,
}

impl Base {
    /// Where this base lives in the complete object at `complete`.
    pub fn address(&self, mem: &dyn Memory, complete: u64) -> Result<u64, Bad> {
        let ct = CatchableType {
            properties: 0,
            type_descriptor: 0,
            mdisp: self.mdisp,
            pdisp: self.pdisp,
            vdisp: self.vdisp,
            size: 0,
            copy: 0,
        };
        adjust(mem, complete, &ct)
    }

    pub fn is_public(&self) -> bool {
        self.attributes & BCD_NOT_PUBLIC == 0
    }
}

/// The locator of the polymorphic object at `object` (through its virtual table pointer).
pub fn locator(mem: &dyn Memory, object: u64) -> Result<Locator, Bad> {
    let vptr = read_u64(mem, object)?;
    let col = read_u64(mem, vptr.checked_sub(8).ok_or(Bad::Overflow)?)?;
    Locator::read(mem, col)
}

/// The complete object that the polymorphic subobject at `object` belongs to, and its locator.
pub fn complete_object(mem: &dyn Memory, object: u64) -> Result<(u64, Locator), Bad> {
    let col = locator(mem, object)?;
    let mut sub = object;
    if col.cd_offset != 0 {
        // A construction displacement stored just before the subobject.
        let d = read_i32(mem, object.checked_sub(u64::from(col.cd_offset)).ok_or(Bad::Overflow)?)?;
        sub = offset(sub, -i64::from(d))?;
    }
    Ok((sub.checked_sub(u64::from(col.offset)).ok_or(Bad::Overflow)?, col))
}

/// The type descriptor (absolute) of the most derived type of the object at `object`
/// (`typeid` of a polymorphic glvalue).
pub fn dynamic_type(mem: &dyn Memory, object: u64) -> Result<u64, Bad> {
    let col = locator(mem, object)?;
    at(col.image_base(), col.type_descriptor, 0)
}

/// The base class array of the hierarchy a locator names: the complete class first.
pub fn bases(mem: &dyn Memory, col: &Locator) -> Result<Vec<Base>, Bad> {
    let base = col.image_base();
    let chd = at(base, col.class_descriptor, 0)?;
    let n = read_u32(mem, chd + 8)?;
    if n > MAX_ENTRIES {
        return Err(Bad::Count(i64::from(n)));
    }
    let array = at(base, read_u32(mem, chd + 12)?, 0)?;
    let mut out = Vec::new();
    for i in 0..u64::from(n) {
        let bcd = at(base, read_u32(mem, array + 4 * i)?, 0)?;
        out.push(Base {
            type_descriptor: at(base, read_u32(mem, bcd)?, 0)?,
            contained: read_u32(mem, bcd + 4)?,
            mdisp: read_i32(mem, bcd + 8)?,
            pdisp: read_i32(mem, bcd + 12)?,
            vdisp: read_i32(mem, bcd + 16)?,
            attributes: read_u32(mem, bcd + 20)?,
        });
    }
    Ok(out)
}

/// `dynamic_cast<Target*>(src)` as the compiler calls it: `vfptr` is the address of the
/// polymorphic subobject that holds the virtual table pointer, and the source pointer (of the
/// type whose descriptor is `source`) is `vfptr - vf_delta` (observed: a `V1*` whose virtual
/// table pointer is in its virtual base `VA` is passed as the `VA` subobject with the distance
/// back to the `V1`). `None`: the cast fails (null, or `std::bad_cast` for a reference).
pub fn dynamic_cast(mem: &dyn Memory, vfptr: u64, vf_delta: i32, source: u64, target: u64) -> Result<Option<u64>, Bad> {
    if vfptr == 0 {
        return Ok(None);
    }
    let object = offset(vfptr, -i64::from(vf_delta))?;
    let (complete, col) = complete_object(mem, vfptr)?;
    let bases = bases(mem, &col)?;
    let mut addrs = Vec::with_capacity(bases.len());
    for b in &bases {
        addrs.push(b.address(mem, complete)?);
    }
    // The source subobject: the entry of the source type at the pointer's address, else the
    // first entry of the source type.
    let mut src = None;
    for (i, b) in bases.iter().enumerate() {
        if same_name(mem, b.type_descriptor, source)? {
            if addrs[i] == object {
                src = Some(i);
                break;
            }
            if src.is_none() {
                src = Some(i);
            }
        }
    }
    let Some(s) = src else { return Ok(None) };
    let src_public = bases[s].is_public();
    let mut targets = Vec::new();
    for (i, b) in bases.iter().enumerate() {
        if same_name(mem, b.type_descriptor, target)? {
            targets.push(i);
        }
    }
    // A downcast: the innermost target that contains the source subobject.
    for &t in targets.iter().rev() {
        let end = t.saturating_add(bases[t].contained as usize);
        if t <= s && s <= end {
            let path_public = if t == 0 { src_public } else { src_public || (s != t && public_within(&bases, t, s)) };
            return Ok(if path_public && (t == 0 || bases[t].is_public()) { Some(addrs[t]) } else { None });
        }
    }
    // Sideways: a target that occurs once (one address), publicly.
    if !src_public {
        return Ok(None);
    }
    let mut found: Option<u64> = None;
    for &t in &targets {
        if !bases[t].is_public() {
            continue;
        }
        match found {
            Some(a) if a != addrs[t] => return Ok(None),
            _ => found = Some(addrs[t]),
        }
    }
    Ok(found)
}

/// Whether base `s` is reached from base `t` (which contains it) through public bases only:
/// every entry between them that contains `s` is public.
fn public_within(bases: &[Base], t: usize, s: usize) -> bool {
    (t + 1..=s).all(|i| {
        let end = i.saturating_add(bases[i].contained as usize);
        !(i <= s && s <= end) || bases[i].is_public()
    })
}

/// One class of a single-inheritance chain of exception classes that a runtime DLL throws
/// itself (`std::bad_cast : std::exception`): its decorated name, its virtual functions and
/// the function that copies an object of it (`copy(this, from)`), all absolute addresses.
#[derive(Debug)]
pub struct ChainClass<'a> {
    pub name: &'a [u8],
    pub virtuals: &'a [u64],
    pub copy: u64,
}

/// What [`write_chain`] made: per class (most derived first) its type descriptor and its
/// virtual table (the address of its first function slot), and the most derived class's
/// ThrowInfo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chain {
    pub type_descriptors: Vec<u64>,
    pub vtables: Vec<u64>,
    pub throw_info: u64,
}

/// Write the records a thrown object of `classes[0]` needs (the RTTI records of the module
/// docs, one locator and virtual table per class, and the throw records of
/// `_CxxThrowException.yaml`: catchable types for every class, most derived first) into `buf`,
/// which lies at `addr` inside the image at `image_base`; every rva is from `image_base`.
/// `type_info_vtable` is put in every type descriptor; `size` is the objects' size and
/// `destructor` the ThrowInfo's destructor. Returns what was made and the bytes used.
pub fn write_chain(
    buf: &mut [u8],
    addr: u64,
    image_base: u64,
    type_info_vtable: u64,
    classes: &[ChainClass],
    size: u32,
    destructor: u64,
) -> Result<(Chain, usize), Bad> {
    struct W<'b> {
        buf: &'b mut [u8],
        used: usize,
        addr: u64,
        base: u64,
    }
    impl W<'_> {
        fn align(&mut self, a: usize) {
            self.used = self.used.div_ceil(a) * a;
        }
        fn put(&mut self, b: &[u8]) -> Result<u64, Bad> {
            let end = self.used.checked_add(b.len()).ok_or(Bad::Overflow)?;
            self.buf.get_mut(self.used..end).ok_or(Bad::Count(end as i64))?.copy_from_slice(b);
            let at = self.addr + self.used as u64;
            self.used = end;
            Ok(at)
        }
        fn rva(&self, a: u64) -> Result<u32, Bad> {
            a.checked_sub(self.base).and_then(|r| u32::try_from(r).ok()).ok_or(Bad::Overflow)
        }
        fn words(&mut self, w: &[u32]) -> Result<u64, Bad> {
            self.align(8);
            let b: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
            self.put(&b)
        }
    }
    let n = classes.len();
    if n == 0 {
        return Err(Bad::Count(0));
    }
    let mut w = W { buf, used: 0, addr, base: image_base };
    let mut tds = Vec::new();
    for c in classes {
        w.align(16);
        let mut b = Vec::with_capacity(17 + c.name.len());
        b.extend_from_slice(&type_info_vtable.to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
        b.extend_from_slice(c.name);
        b.push(0);
        tds.push(w.put(&b)?);
    }
    // Class hierarchy descriptors (their base array filled below), then base descriptors.
    let mut chds = Vec::new();
    for i in 0..n {
        chds.push(w.words(&[0, 0, (n - i) as u32, 0])?);
    }
    let mut bcds = Vec::new();
    for j in 0..n {
        let (td, chd) = (w.rva(tds[j])?, w.rva(chds[j])?);
        bcds.push(w.words(&[td, (n - 1 - j) as u32, 0, u32::MAX, 0, 0x40, chd])?);
    }
    for i in 0..n {
        let rvas: Result<Vec<u32>, Bad> = bcds[i..].iter().map(|&b| w.rva(b)).collect();
        let array = w.words(&rvas?)?;
        let at = (chds[i] - addr) as usize + 12;
        let r = w.rva(array)?;
        w.buf[at..at + 4].copy_from_slice(&r.to_le_bytes());
    }
    let mut vtables = Vec::new();
    for (i, c) in classes.iter().enumerate() {
        let col = w.words(&[1, 0, 0, w.rva(tds[i])?, w.rva(chds[i])?, 0])?;
        let r = w.rva(col)?;
        let at = (col - addr) as usize + 20;
        w.buf[at..at + 4].copy_from_slice(&r.to_le_bytes());
        w.align(8);
        w.put(&col.to_le_bytes())?;
        let mut vt = Vec::new();
        for &f in c.virtuals {
            vt.extend_from_slice(&f.to_le_bytes());
        }
        vtables.push(w.addr + w.used as u64);
        w.put(&vt)?;
    }
    let mut cts = Vec::new();
    for (i, c) in classes.iter().enumerate() {
        let (td, copy) = (w.rva(tds[i])?, w.rva(c.copy)?);
        cts.push(w.words(&[0, td, 0, u32::MAX, 0, size, copy])?);
    }
    let mut cta = vec![n as u32];
    for &ct in &cts {
        cta.push(w.rva(ct)?);
    }
    let cta = w.words(&cta)?;
    let throw_info = w.words(&[0, w.rva(destructor)?, 0, w.rva(cta)?])?;
    Ok((Chain { type_descriptors: tds, vtables, throw_info }, w.used))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SliceMemory;
    use alloc::vec;

    const BASE: u64 = 0x1_8000_0000;

    struct Img(Vec<u8>);

    impl Img {
        fn put(&mut self, b: &[u8]) -> u32 {
            while self.0.len() % 8 != 0 {
                self.0.push(0);
            }
            let r = self.0.len() as u32;
            self.0.extend_from_slice(b);
            r
        }
        fn words(&mut self, w: &[u32]) -> u32 {
            let b: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
            self.put(&b)
        }
        fn td(&mut self, name: &str) -> u32 {
            let mut b = vec![0u8; 16];
            b.extend_from_slice(name.as_bytes());
            b.push(0);
            self.put(&b)
        }
        fn set_u64(&mut self, rva: u32, v: u64) {
            self.0[rva as usize..rva as usize + 8].copy_from_slice(&v.to_le_bytes());
        }
    }

    /// (type descriptor rva, contained, mdisp, pdisp, vdisp, attributes)
    type B = (u32, u32, i32, i32, i32, u32);

    /// A hierarchy and one virtual table per (subobject offset), returning the vtable rvas.
    fn class(img: &mut Img, td: u32, attrs: u32, bases: &[B], offsets: &[u32]) -> Vec<u32> {
        let chd = img.words(&[0, attrs, bases.len() as u32, 0]);
        let mut rvas = Vec::new();
        for &(t, c, m, p, v, a) in bases {
            rvas.push(img.words(&[t, c, m as u32, p as u32, v as u32, a, chd]));
        }
        let array = img.words(&rvas);
        img.0[chd as usize + 12..chd as usize + 16].copy_from_slice(&array.to_le_bytes());
        let mut vtables = Vec::new();
        for &off in offsets {
            let col = img.words(&[1, off, 0, td, chd, 0]);
            img.0[col as usize + 20..col as usize + 24].copy_from_slice(&col.to_le_bytes());
            // vptr[-1] = the locator, vptr[0] = a slot.
            let slot = img.put(&[0u8; 16]);
            img.set_u64(slot, BASE + u64::from(col));
            vtables.push(slot + 8);
        }
        vtables
    }

    /// `D : B, C` with `B : A`, `C : A` (A twice), as the sheet's records give it.
    #[test]
    fn diamond_without_virtual_bases() {
        let mut img = Img(vec![0; 0x100]);
        let (a, b, c, d, x) =
            (img.td(".?AUA@@"), img.td(".?AUB@@"), img.td(".?AUC@@"), img.td(".?AUD@@"), img.td(".?AUX@@"));
        let vt = class(
            &mut img,
            d,
            0x5,
            &[
                (d, 4, 0, -1, 0, 0x40),
                (b, 1, 0, -1, 0, 0x40),
                (a, 0, 0, -1, 0, 0x42),
                (c, 1, 24, -1, 0, 0x40),
                (a, 0, 24, -1, 0, 0x42),
            ],
            &[0, 24],
        );
        // The object: vptr at 0 (D/B/A), vptr at 24 (C/A).
        let obj = img.put(&[0u8; 48]);
        img.set_u64(obj, BASE + u64::from(vt[0]));
        img.set_u64(obj + 24, BASE + u64::from(vt[1]));
        let m = SliceMemory { base: BASE, bytes: &img.0 };
        let o = BASE + u64::from(obj);
        let td = |r: u32| BASE + u64::from(r);
        assert_eq!(complete_object(&m, o + 24).unwrap().0, o);
        assert_eq!(dynamic_type(&m, o + 24).unwrap(), td(d));
        assert_eq!(dynamic_cast(&m, o, 0, td(b), td(d)).unwrap(), Some(o));
        assert_eq!(dynamic_cast(&m, o + 24, 0, td(c), td(d)).unwrap(), Some(o));
        assert_eq!(dynamic_cast(&m, o + 24, 0, td(a), td(d)).unwrap(), Some(o), "the A in C -> D");
        assert_eq!(dynamic_cast(&m, o, 0, td(b), td(c)).unwrap(), Some(o + 24), "sideways");
        assert_eq!(dynamic_cast(&m, o, 0, td(a), td(c)).unwrap(), Some(o + 24), "from the ambiguous A in B to C");
        assert_eq!(dynamic_cast(&m, o + 24, 0, td(a), td(b)).unwrap(), Some(o));
        assert_eq!(dynamic_cast(&m, o, 0, td(a), td(b)).unwrap(), Some(o));
        assert_eq!(dynamic_cast(&m, o, 0, td(b), td(x)).unwrap(), None);
        assert_eq!(dynamic_cast(&m, 0, 0, td(b), td(d)).unwrap(), None);
        assert_eq!(dynamic_cast(&m, o, 0, td(d), td(d)).unwrap(), Some(o));
        // A to the ambiguous A itself from B: two addresses, so none sideways (but a downcast
        // from the A in B finds the A in B).
        assert_eq!(dynamic_cast(&m, o, 0, td(b), td(a)).unwrap(), None);
    }

    /// `PrivB : private A`: the A is not a public path.
    #[test]
    fn private_bases_are_not_paths() {
        let mut img = Img(vec![0; 0x100]);
        let (a, p) = (img.td(".?AUA@@"), img.td(".?AUPrivB@@"));
        let vt = class(&mut img, p, 0, &[(p, 1, 0, -1, 0, 0x40), (a, 0, 0, -1, 0, 0x4d)], &[0]);
        let obj = img.put(&[0u8; 16]);
        img.set_u64(obj, BASE + u64::from(vt[0]));
        let m = SliceMemory { base: BASE, bytes: &img.0 };
        let o = BASE + u64::from(obj);
        assert_eq!(dynamic_cast(&m, o, 0, BASE + u64::from(a), BASE + u64::from(p)).unwrap(), None);
    }

    /// `VD : V1, V2`, both `: virtual VA`: VA at 40, V2 at 16, through V1's vbtable.
    #[test]
    fn virtual_bases_go_through_the_vbtable() {
        let mut img = Img(vec![0; 0x100]);
        let (va, v1, v2, vd) = (img.td(".?AUVA@@"), img.td(".?AUV1@@"), img.td(".?AUV2@@"), img.td(".?AUVD@@"));
        let vt = class(
            &mut img,
            vd,
            0x3,
            &[
                (vd, 4, 0, -1, 0, 0x40),
                (v1, 1, 0, -1, 0, 0x40),
                (va, 0, 0, 0, 4, 0x50),
                (v2, 1, 16, -1, 0, 0x40),
                (va, 0, 0, 0, 4, 0x50),
            ],
            &[40],
        );
        // vbtables: V1's at offset 0 of VD says VA is at +40; V2's says +24 (from 16).
        let vb1 = img.words(&[0, 40]);
        let obj = img.put(&[0u8; 56]);
        img.set_u64(obj, BASE + u64::from(vb1));
        img.set_u64(obj + 40, BASE + u64::from(vt[0]));
        let m = SliceMemory { base: BASE, bytes: &img.0 };
        let o = BASE + u64::from(obj);
        let td = |r: u32| BASE + u64::from(r);
        assert_eq!(complete_object(&m, o + 40).unwrap().0, o);
        assert_eq!(dynamic_cast(&m, o + 40, 0, td(va), td(vd)).unwrap(), Some(o));
        assert_eq!(dynamic_cast(&m, o + 40, 0, td(va), td(v1)).unwrap(), Some(o));
        assert_eq!(dynamic_cast(&m, o + 40, 0, td(va), td(v2)).unwrap(), Some(o + 16));
        assert_eq!(dynamic_type(&m, o + 40).unwrap(), td(vd));
        // V1* (at 0, no virtual table pointer of its own) -> V2*: passed as the VA subobject
        // and the distance back to the V1.
        assert_eq!(dynamic_cast(&m, o + 40, 40, td(v1), td(v2)).unwrap(), Some(o + 16));
        assert_eq!(dynamic_cast(&m, o + 40, 24, td(v2), td(v1)).unwrap(), Some(o));
        assert_eq!(dynamic_cast(&m, o + 40, 24, td(v2), td(vd)).unwrap(), Some(o));
    }

    /// A chain written for a runtime DLL reads back as the compiler's records do.
    #[test]
    fn written_chains_read_back() {
        let mut img = Img(vec![0; 0x1000]);
        let start = 0x100u64;
        let addr = BASE + start;
        let classes = [
            ChainClass { name: b".?AVbad_cast@std@@", virtuals: &[BASE + 0x10, BASE + 0x20], copy: BASE + 0x30 },
            ChainClass { name: b".?AVexception@std@@", virtuals: &[BASE + 0x10, BASE + 0x28], copy: BASE + 0x38 },
        ];
        let (chain, used) =
            write_chain(&mut img.0[start as usize..], addr, BASE, BASE + 0x40, &classes, 24, BASE + 0x48).unwrap();
        assert!(used < 0x600);
        // An object of the most derived class.
        let obj = img.put(&[0u8; 24]);
        img.set_u64(obj, chain.vtables[0]);
        let m = SliceMemory { base: BASE, bytes: &img.0 };
        let o = BASE + u64::from(obj);
        assert_eq!(dynamic_type(&m, o).unwrap(), chain.type_descriptors[0]);
        assert_eq!(read_u64(&m, chain.vtables[0] + 8).unwrap(), BASE + 0x20);
        assert_eq!(dynamic_cast(&m, o, 0, chain.type_descriptors[1], chain.type_descriptors[0]).unwrap(), Some(o));
        assert_eq!(read_u64(&m, chain.type_descriptors[0]).unwrap(), BASE + 0x40);
        // The throw records: both classes catchable, the copy functions and size kept.
        let exc = crate::throw::Exception::new(
            crate::consts::CXX_EXCEPTION,
            1,
            &[u64::from(crate::consts::MAGIC_V1), o, chain.throw_info, BASE],
        );
        let thrown = crate::throw::Thrown::read(&m, &exc).unwrap();
        assert_eq!(thrown.types.len(), 2);
        assert_eq!(thrown.types[1].copy, 0x38);
        assert_eq!(thrown.types[0].size, 24);
        assert_eq!(thrown.info.destructor, 0x48);
        assert!(same_name(&m, BASE + u64::from(thrown.types[1].type_descriptor), chain.type_descriptors[1]).unwrap());
        // Too small a buffer is an error, not a panic.
        let mut small = [0u8; 64];
        assert!(write_chain(&mut small, addr, BASE, 0, &classes, 24, BASE).is_err());
    }

    #[test]
    fn unreadable_objects_are_errors() {
        let img = Img(vec![0; 0x40]);
        let m = SliceMemory { base: BASE, bytes: &img.0 };
        assert!(dynamic_type(&m, BASE + 0x10).is_err());
        assert!(dynamic_cast(&m, BASE + 0x10, 0, BASE, BASE).is_err());
    }
}
