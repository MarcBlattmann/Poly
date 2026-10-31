//! What a `throw` raises and how a catch clause takes it (the spec's
//! `_CxxThrowException.yaml` and the rules `match_type`, `this_adjustment`
//! and `catch_object` of `__CxxFrameHandler3.yaml`).
//!
//! ```text
//! record: code 0xE06D7363, 4 parameters: 0x19930520, object, ThrowInfo, image base
//! ThrowInfo          attributes, destructor, forward compat, CatchableTypeArray   (rvas)
//! CatchableTypeArray count, CatchableType rvas...
//! CatchableType      properties, TypeDescriptor, mdisp, pdisp, vdisp, size, copy function
//! TypeDescriptor     vftable pointer (8), spare (8), decorated name (NUL-terminated)
//! ```

use crate::consts::*;
use crate::tables::HandlerType;
use crate::{Bad, Memory, at, count, offset, read_i32, read_u8, read_u32, read_u64};
use alloc::vec::Vec;

/// An exception as the frame handlers see it: the record's code, flags and
/// first four parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exception {
    pub code: u32,
    pub flags: u32,
    pub count: u32,
    pub params: [u64; 4],
}

impl Exception {
    /// From a record's code, flags and parameters (the first four kept).
    pub fn new(code: u32, flags: u32, params: &[u64]) -> Exception {
        let mut p = [0u64; 4];
        let n = params.len().min(4);
        p[..n].copy_from_slice(&params[..n]);
        Exception { code, flags, count: params.len().min(15) as u32, params: p }
    }

    /// A C++ exception: the code, four parameters, a known magic number.
    pub fn is_cxx(&self) -> bool {
        self.code == CXX_EXCEPTION
            && self.count >= 4
            && u32::try_from(self.params[0]).is_ok_and(|m| (MAGIC_V1..=MAGIC_V3).contains(&m))
    }

    /// A rethrow record: a C++ exception without object and ThrowInfo
    /// (`throw;`), not completed yet.
    pub fn is_rethrow(&self) -> bool {
        self.is_cxx() && self.params[1] == 0 && self.params[2] == 0
    }

    pub fn object(&self) -> u64 {
        self.params[1]
    }

    pub fn throw_info(&self) -> u64 {
        self.params[2]
    }

    pub fn image_base(&self) -> u64 {
        self.params[3]
    }

    /// Complete a rethrow record with the exception being handled (`current`,
    /// if it is a C++ exception): object, ThrowInfo and image base. True if
    /// it did.
    pub fn complete_rethrow(&mut self, current: Option<&Exception>) -> bool {
        match current {
            Some(c) if self.is_rethrow() && c.is_cxx() && !c.is_rethrow() => {
                self.params[1..4].copy_from_slice(&c.params[1..4]);
                true
            }
            _ => false,
        }
    }
}

/// `ThrowInfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThrowInfo {
    pub attributes: u32,
    pub destructor: u32,
    pub forward_compat: u32,
    pub catchable_types: u32,
}

impl ThrowInfo {
    pub fn read(mem: &dyn Memory, addr: u64) -> Result<ThrowInfo, Bad> {
        let f = |i: u64| read_u32(mem, addr.checked_add(4 * i).ok_or(Bad::Overflow)?);
        Ok(ThrowInfo { attributes: f(0)?, destructor: f(1)?, forward_compat: f(2)?, catchable_types: f(3)? })
    }
}

/// `CatchableType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CatchableType {
    pub properties: u32,
    pub type_descriptor: u32,
    pub mdisp: i32,
    pub pdisp: i32,
    pub vdisp: i32,
    pub size: i32,
    pub copy: u32,
}

impl CatchableType {
    pub fn read(mem: &dyn Memory, addr: u64) -> Result<CatchableType, Bad> {
        let f = |i: u64| read_u32(mem, addr.checked_add(4 * i).ok_or(Bad::Overflow)?);
        Ok(CatchableType {
            properties: f(0)?,
            type_descriptor: f(1)?,
            mdisp: f(2)? as i32,
            pdisp: f(3)? as i32,
            vdisp: f(4)? as i32,
            size: f(5)? as i32,
            copy: f(6)?,
        })
    }
}

/// The thrown type's ThrowInfo and catchable types (in array order).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thrown {
    pub info: ThrowInfo,
    pub types: Vec<CatchableType>,
}

impl Thrown {
    /// Read them through the record of a (completed) C++ exception.
    pub fn read(mem: &dyn Memory, exc: &Exception) -> Result<Thrown, Bad> {
        let base = exc.image_base();
        let info = ThrowInfo::read(mem, exc.throw_info())?;
        let array = at(base, info.catchable_types, 0)?;
        let n = read_i32(mem, array)?;
        let n = count(u32::try_from(n).map_err(|_| Bad::Count(i64::from(n)))?)?;
        let mut types = Vec::new();
        for i in 0..n {
            let rva = read_u32(mem, array + 4 + 4 * u64::from(i))?;
            types.push(CatchableType::read(mem, at(base, rva, 0)?)?);
        }
        Ok(Thrown { info, types })
    }

    /// The exception object's destructor (absolute), if it has one.
    pub fn destructor(&self, exc: &Exception) -> Result<Option<u64>, Bad> {
        if self.info.destructor == 0 { Ok(None) } else { at(exc.image_base(), self.info.destructor, 0).map(Some) }
    }
}

/// The type descriptors at `a` and `b` have the same decorated name (byte
/// for byte; the vftable pointer is not read).
pub fn same_name(mem: &dyn Memory, a: u64, b: u64) -> Result<bool, Bad> {
    if a == b {
        return Ok(true);
    }
    for i in 0..MAX_NAME {
        let x = read_u8(mem, a.checked_add(16 + i).ok_or(Bad::Overflow)?)?;
        let y = read_u8(mem, b.checked_add(16 + i).ok_or(Bad::Overflow)?)?;
        if x != y {
            return Ok(false);
        }
        if x == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// How a clause takes an exception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    /// `catch(...)`.
    CatchAll,
    /// A typed clause, through this catchable type of the thrown array.
    Typed(CatchableType),
}

/// Does clause `h` (of a function in the image at `frame_base`) take the
/// completed C++ exception `exc` of type `thrown`? A typed clause matches
/// the first catchable type with its decorated name, if the throw's
/// qualifiers are all among the clause's adjectives.
pub fn match_clause(
    mem: &dyn Memory,
    frame_base: u64,
    h: &HandlerType,
    exc: &Exception,
    thrown: &Thrown,
) -> Result<Option<Match>, Bad> {
    if h.type_descriptor == 0 {
        return Ok(Some(Match::CatchAll));
    }
    if thrown.info.attributes & TI_QUALIFIERS & !h.adjectives != 0 {
        return Ok(None);
    }
    let clause = at(frame_base, h.type_descriptor, 0)?;
    for ct in &thrown.types {
        if same_name(mem, clause, at(exc.image_base(), ct.type_descriptor, 0)?)? {
            return Ok(Some(Match::Typed(*ct)));
        }
    }
    Ok(None)
}

/// The address of the matched type inside the object at `object`: through
/// the virtual base table if the type is in a virtual base (`pdisp >= 0`),
/// then `mdisp`.
pub fn adjust(mem: &dyn Memory, object: u64, ct: &CatchableType) -> Result<u64, Bad> {
    let mut p = object;
    if ct.pdisp >= 0 {
        let vbptr = offset(object, i64::from(ct.pdisp))?;
        let vbtable = read_u64(mem, vbptr)?;
        let vb = read_i32(mem, offset(vbtable, i64::from(ct.vdisp))?)?;
        p = offset(vbptr, i64::from(vb))?;
    }
    offset(p, i64::from(ct.mdisp))
}

/// How the catch object is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatchObject {
    /// No catch object (`catch(...)`, an unnamed parameter).
    None,
    /// Store this 8-byte value at `at`: the (adjusted) address of the
    /// exception object for a reference, or a thrown pointer converted.
    Value { at: u64, value: u64 },
    /// Copy `size` bytes from `from` to `at`.
    Bytes { at: u64, from: u64, size: u64 },
    /// Call the copy function: `function(at, from)`, with a third argument
    /// 1 when the class has virtual bases.
    Copy { at: u64, from: u64, function: u64, virtual_bases: bool },
}

/// The catch object clause `h` makes from the completed C++ exception
/// `exc`, matched as `m`, in the frame `parent_frame` (the function's
/// establisher frame).
pub fn catch_object(
    mem: &dyn Memory,
    h: &HandlerType,
    m: &Match,
    exc: &Exception,
    parent_frame: u64,
) -> Result<CatchObject, Bad> {
    let Match::Typed(ct) = m else { return Ok(CatchObject::None) };
    if h.catch_obj_offset == 0 {
        return Ok(CatchObject::None);
    }
    let dest = offset(parent_frame, i64::from(h.catch_obj_offset))?;
    let object = exc.object();
    if h.adjectives & ADJ_REFERENCE != 0 {
        return Ok(CatchObject::Value { at: dest, value: adjust(mem, object, ct)? });
    }
    let size = u64::try_from(ct.size).map_err(|_| Bad::Count(i64::from(ct.size)))?;
    if ct.properties & CT_SIMPLE != 0 {
        if size == 8 {
            // A pointer is converted like an object address (null stays
            // null); a scalar has mdisp 0 and no virtual base.
            let v = read_u64(mem, object)?;
            let value = if v == 0 { 0 } else { adjust(mem, v, ct)? };
            return Ok(CatchObject::Value { at: dest, value });
        }
        return Ok(CatchObject::Bytes { at: dest, from: object, size });
    }
    let from = adjust(mem, object, ct)?;
    if ct.copy != 0 {
        return Ok(CatchObject::Copy {
            at: dest,
            from,
            function: at(exc.image_base(), ct.copy, 0)?,
            virtual_bases: ct.properties & CT_VIRTUAL_BASES != 0,
        });
    }
    Ok(CatchObject::Bytes { at: dest, from, size })
}
