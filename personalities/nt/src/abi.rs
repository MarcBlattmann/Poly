//! The x64 system call convention of `ntdll`'s stubs and the structures
//! the services read and write in user memory.
//!
//! A stub is called with the x64 calling convention (Learn: "x64 calling
//! convention"): the first four arguments in RCX, RDX, R8 and R9, the rest
//! on the stack above 32 bytes of shadow space and the return address. It
//! moves RCX to R10 (`syscall` overwrites RCX with the return address),
//! puts the service number in EAX and executes `syscall`. So at the
//! `syscall` the arguments are R10, RDX, R8, R9, then the quadwords at RSP +
//! 0x28, RSP + 0x30, ... The result is an `NTSTATUS` in EAX.

use crate::consts::*;
use crate::kernel::NtKernel;
use alloc::vec::Vec;
use core::result::Result;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NtStatus};

/// The longest name a `UNICODE_STRING` can hold: `Length` is a `USHORT`
/// count of bytes.
pub const MAX_UNICODE_BYTES: usize = 0xFFFE;

/// Argument `index` (0-based) of the service being called.
pub fn arg<K: NtKernel>(k: &mut K, regs: &SyscallRegs, index: usize) -> Result<u64, NtStatus> {
    Ok(match index {
        0 => regs.r10,
        1 => regs.rdx,
        2 => regs.r8,
        3 => regs.r9,
        _ => {
            let at = regs.rsp.checked_add(0x28 + 8 * (index as u64 - 4)).ok_or(STATUS_ACCESS_VIOLATION)?;
            read_u64(k, at)?
        }
    })
}

/// The first `N` arguments.
pub fn args<K: NtKernel, const N: usize>(k: &mut K, regs: &SyscallRegs) -> Result<[u64; N], NtStatus> {
    let mut out = [0u64; N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = arg(k, regs, i)?;
    }
    Ok(out)
}

pub fn read<K: NtKernel>(k: &mut K, addr: u64, buf: &mut [u8]) -> Result<(), NtStatus> {
    if addr == 0 {
        return Err(STATUS_ACCESS_VIOLATION);
    }
    k.read_user(addr, buf).map_err(|_| STATUS_ACCESS_VIOLATION)
}

pub fn write<K: NtKernel>(k: &mut K, addr: u64, data: &[u8]) -> Result<(), NtStatus> {
    if addr == 0 {
        return Err(STATUS_ACCESS_VIOLATION);
    }
    k.write_user(addr, data).map_err(|_| STATUS_ACCESS_VIOLATION)
}

pub fn read_u64<K: NtKernel>(k: &mut K, addr: u64) -> Result<u64, NtStatus> {
    let mut b = [0u8; 8];
    read(k, addr, &mut b)?;
    Ok(u64::from_le_bytes(b))
}

pub fn write_u64<K: NtKernel>(k: &mut K, addr: u64, value: u64) -> Result<(), NtStatus> {
    write(k, addr, &value.to_le_bytes())
}

pub fn write_u32<K: NtKernel>(k: &mut K, addr: u64, value: u32) -> Result<(), NtStatus> {
    write(k, addr, &value.to_le_bytes())
}

pub fn read_u32<K: NtKernel>(k: &mut K, addr: u64) -> Result<u32, NtStatus> {
    let mut b = [0u8; 4];
    read(k, addr, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// Write `value` to an optional out-parameter (nothing for a null pointer).
pub fn write_opt_u32<K: NtKernel>(k: &mut K, addr: u64, value: u32) -> Result<(), NtStatus> {
    if addr == 0 { Ok(()) } else { write_u32(k, addr, value) }
}

/// A `UNICODE_STRING` header (`ntdef.h`): `Length` and `MaximumLength` in
/// bytes, then the buffer pointer at offset 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: u64,
}

impl UnicodeString {
    pub fn to_bytes(&self) -> [u8; UNICODE_STRING_SIZE] {
        let mut b = [0u8; UNICODE_STRING_SIZE];
        b[0..2].copy_from_slice(&self.length.to_le_bytes());
        b[2..4].copy_from_slice(&self.maximum_length.to_le_bytes());
        b[8..16].copy_from_slice(&self.buffer.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; UNICODE_STRING_SIZE]) -> UnicodeString {
        UnicodeString {
            length: u16::from_le_bytes([b[0], b[1]]),
            maximum_length: u16::from_le_bytes([b[2], b[3]]),
            buffer: u64::from_le_bytes(b[8..16].try_into().expect("8 bytes")),
        }
    }
}

pub fn read_unicode_header<K: NtKernel>(k: &mut K, addr: u64) -> Result<UnicodeString, NtStatus> {
    let mut b = [0u8; UNICODE_STRING_SIZE];
    read(k, addr, &mut b)?;
    Ok(UnicodeString::from_bytes(&b))
}

/// Capture the characters of the `UNICODE_STRING` at `addr`. An odd
/// `Length`, or one above `MaximumLength`, is `STATUS_INVALID_PARAMETER`
/// (Learn, "UNICODE_STRING": the length is in bytes, the maximum is the
/// buffer's size; the status is to be confirmed by the spec agent).
pub fn read_unicode<K: NtKernel>(k: &mut K, addr: u64) -> Result<Vec<u16>, NtStatus> {
    let s = read_unicode_header(k, addr)?;
    if s.length % 2 != 0 || s.length > s.maximum_length {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if s.length == 0 {
        return Ok(Vec::new());
    }
    let mut bytes = alloc::vec![0u8; s.length as usize];
    read(k, s.buffer, &mut bytes)?;
    Ok(bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

/// An `OBJECT_ATTRIBUTES` (`ntdef.h`) copied in: root directory, name and
/// `OBJ_*` flags. The security descriptor and quality of service are
/// ignored until security exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    pub root: Option<Handle>,
    pub name: Option<Vec<u16>>,
    pub attributes: u32,
}

impl Attributes {
    pub fn as_object_attributes(&self) -> poly_ntobj::ObjectAttributes<'_> {
        poly_ntobj::ObjectAttributes { root: self.root, name: self.name.as_deref(), attributes: self.attributes }
    }
}

/// Capture the `OBJECT_ATTRIBUTES` at `addr` (a null pointer is "no
/// attributes": an unnamed object). `Length` must be
/// `sizeof(OBJECT_ATTRIBUTES)` (Learn: "OBJECT_ATTRIBUTES", "the number of
/// bytes of data contained in this structure"; `STATUS_INVALID_PARAMETER`
/// otherwise, to be confirmed by the spec agent).
pub fn read_attributes<K: NtKernel>(k: &mut K, addr: u64) -> Result<Attributes, NtStatus> {
    if addr == 0 {
        return Ok(Attributes::default());
    }
    let mut b = [0u8; OBJECT_ATTRIBUTES_SIZE];
    read(k, addr, &mut b)?;
    let q = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"));
    let length = u32::from_le_bytes(b[0..4].try_into().expect("4 bytes"));
    if length as usize != OBJECT_ATTRIBUTES_SIZE {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let root = q(8);
    let name = q(16);
    let attributes = u32::from_le_bytes(b[24..28].try_into().expect("4 bytes"));
    let name = if name == 0 { None } else { Some(read_unicode(k, name)?) };
    Ok(Attributes { root: (root != 0).then_some(Handle(root)), name, attributes })
}

/// UTF-16 code units as little-endian bytes.
pub fn utf16_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}
