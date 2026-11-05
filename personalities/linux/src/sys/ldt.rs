//! modify_ldt(2): the process's local descriptor table (ADR 0134).
//!
//! Poly keeps the table per process ([`crate::LinuxProcess::ldt`]): its
//! threads share it, fork(2) copies it, execve(2) drops it. The raw
//! descriptors are those the spec sheet records (`asm/ldt.h`: 8192
//! entries of 8 bytes, `struct user_desc`). The CPU is not given the
//! table: loading a selector that points into it faults (`SIGSEGV`), as
//! loading a cleared entry does on Linux.
//!
//! The result is an `int` the kernel zero-extends: errors reach user space
//! as the 32-bit `-errno` (`0xffffffda` for `ENOSYS`).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};
use alloc::vec;

pub const CALLS: &[(u64, Handler)] = &[(nr::MODIFY_LDT, modify_ldt)];

/// `LDT_ENTRIES`, `LDT_ENTRY_SIZE` (`asm/ldt.h`).
pub const LDT_ENTRIES: u32 = 8192;
pub const LDT_ENTRY_SIZE: u64 = 8;
/// The most func 0 copies: the whole table.
const READ_MAX: u64 = LDT_ENTRIES as u64 * LDT_ENTRY_SIZE;
/// What func 2 reads of the default LDT, as the sheet records.
const DEFAULT_LDT_BYTES: u64 = 128;

/// `struct user_desc` flag bits, in declaration order (`asm/ldt.h`).
const SEG_32BIT: u32 = 1 << 0;
const CONTENTS_SHIFT: u32 = 1;
const READ_EXEC_ONLY: u32 = 1 << 3;
const LIMIT_IN_PAGES: u32 = 1 << 4;
const SEG_NOT_PRESENT: u32 = 1 << 5;
const USEABLE: u32 = 1 << 6;
const LM: u32 = 1 << 7;

/// `struct user_desc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserDesc {
    pub entry: u32,
    pub base: u32,
    pub limit: u32,
    pub flags: u32,
}

impl UserDesc {
    pub fn from_bytes(b: &[u8; 16]) -> UserDesc {
        let w = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        UserDesc { entry: w(0), base: w(1), limit: w(2), flags: w(3) }
    }

    const fn contents(&self) -> u32 {
        (self.flags >> CONTENTS_SHIFT) & 3
    }

    /// An "empty" descriptor clears its entry: read-exec-only and not
    /// present with nothing else set (in the old mode also base and limit
    /// 0).
    pub const fn is_empty(&self, old_mode: bool) -> bool {
        let only = self.flags & 0xff == READ_EXEC_ONLY | SEG_NOT_PRESENT;
        only && (!old_mode || (self.base == 0 && self.limit == 0))
    }

    /// The 8-byte segment descriptor: limit 0..15, base 0..23, access byte
    /// (present unless `seg_not_present`, DPL 3, S, type: accessed,
    /// writable/readable unless `read_exec_only`, `contents`), limit
    /// 16..19, flags (granularity, D/B, L, AVL in the new mode only), base
    /// 24..31.
    pub const fn descriptor(&self, old_mode: bool) -> u64 {
        let (base, limit, f) = (self.base as u64, self.limit as u64, self.flags);
        let mut ty = 1 | ((self.contents() as u64) << 2);
        if f & READ_EXEC_ONLY == 0 {
            ty |= 2;
        }
        let present = if f & SEG_NOT_PRESENT == 0 { 0x80 } else { 0 };
        let access = present | 0x60 | 0x10 | ty;
        let mut nibble = 0;
        if f & USEABLE != 0 && !old_mode {
            nibble |= 1;
        }
        if f & LM != 0 {
            nibble |= 2;
        }
        if f & SEG_32BIT != 0 {
            nibble |= 4;
        }
        if f & LIMIT_IN_PAGES != 0 {
            nibble |= 8;
        }
        (limit & 0xffff)
            | ((base & 0xff_ffff) << 16)
            | (access << 40)
            | (((limit >> 16) & 0xf) << 48)
            | (nibble << 52)
            | (((base >> 24) & 0xff) << 56)
    }
}

/// modify_ldt(func, ptr, bytecount).
pub fn modify_ldt(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let r = match a.int(0) {
        0 => read(k, a.ptr(1), a.ulong(2)),
        1 => write(k, a.ptr(1), a.ulong(2), true),
        2 => read_default(k, a.ptr(1), a.ulong(2)),
        0x11 => write(k, a.ptr(1), a.ulong(2), false),
        _ => Err(Errno::ENOSYS),
    };
    // An int, zero-extended.
    Ok(match r {
        Ok(v) => v,
        Err(e) => u64::from(e.to_rax() as u32),
    })
}

/// func 0: the table's raw descriptors, zero-filled to `bytecount` (at
/// most the whole table); 0 without copying anything for no table.
fn read(k: &mut dyn LinuxKernel, ptr: UserPtr, bytecount: u64) -> SysResult {
    let ldt = with_own(k, |p| p.ldt.clone());
    if ldt.is_empty() {
        return Ok(0);
    }
    let n = bytecount.min(READ_MAX) as usize;
    let mut bytes = vec![0u8; n];
    for (i, d) in ldt.iter().enumerate() {
        let at = i * 8;
        if at >= n {
            break;
        }
        let m = (n - at).min(8);
        bytes[at..at + m].copy_from_slice(&d.to_le_bytes()[..m]);
    }
    k.copy_to_user(ptr, &bytes)?;
    Ok(n as u64)
}

/// func 2: the default LDT, all zeros.
fn read_default(k: &mut dyn LinuxKernel, ptr: UserPtr, bytecount: u64) -> SysResult {
    let n = bytecount.min(DEFAULT_LDT_BYTES) as usize;
    k.copy_to_user(ptr, &vec![0u8; n])?;
    Ok(n as u64)
}

/// func 1 (old mode) and 0x11: install one entry.
fn write(k: &mut dyn LinuxKernel, ptr: UserPtr, bytecount: u64, old_mode: bool) -> SysResult {
    if bytecount != 16 {
        return Err(Errno::EINVAL);
    }
    let mut b = [0u8; 16];
    k.copy_from_user(ptr, &mut b)?;
    let d = UserDesc::from_bytes(&b);
    if d.entry >= LDT_ENTRIES {
        return Err(Errno::EINVAL);
    }
    let empty = d.is_empty(old_mode);
    // Conforming code segments are refused unless not present.
    if !empty && d.contents() == 3 && d.flags & SEG_NOT_PRESENT == 0 {
        return Err(Errno::EINVAL);
    }
    let value = if empty { 0 } else { d.descriptor(old_mode) };
    with_own(k, |p| {
        let i = d.entry as usize;
        if p.ldt.len() <= i {
            p.ldt.resize(i + 1, 0);
        }
        p.ldt[i] = value;
    });
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_as_the_sheet_records() {
        let flat = UserDesc { entry: 0, base: 0, limit: 0xfffff, flags: SEG_32BIT | LIMIT_IN_PAGES };
        assert_eq!(flat.descriptor(false), 0x00cf_f300_0000_ffff);
        let u = UserDesc { entry: 2, base: 0x1234_5678, limit: 0xabcde, flags: SEG_32BIT | USEABLE };
        assert_eq!(u.descriptor(false), 0x125a_f334_5678_bcde);
        assert_eq!(u.descriptor(true), 0x124a_f334_5678_bcde, "old mode clears AVL");
        let e = UserDesc { entry: 0, base: 0, limit: 0, flags: READ_EXEC_ONLY | SEG_NOT_PRESENT };
        assert!(e.is_empty(false) && e.is_empty(true));
        assert!(!UserDesc { base: 1, ..e }.is_empty(true));
    }
}
