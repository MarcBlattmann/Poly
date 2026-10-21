//! The loader's lists in process memory: `PEB.Ldr` (`PEB_LDR_DATA`) and one
//! `LDR_DATA_TABLE_ENTRY` per module, linked by `LIST_ENTRY`s (`winternl.h`,
//! `ntdef.h`; offsets for x64).
//!
//! Documented fields: `PEB.Ldr` (0x18), `PEB_LDR_DATA.InMemoryOrderModuleList`
//! (0x20), and in each entry `InMemoryOrderLinks` (0x10), `DllBase` (0x30),
//! `EntryPoint` (0x38), `FullDllName` (0x48), `CheckSum` (0x78) and
//! `TimeDateStamp` (0x80). Poly's own use of reserved space (ADR 0071, to be
//! compared with Windows by the spec agent): the in-load-order links in the
//! entry's `Reserved1` (0x00) with their list head in `PEB_LDR_DATA`'s
//! `Reserved2` (0x10); the in-initialisation-order links in `Reserved2`
//! (0x20) with their head after the documented structure (0x30); the image
//! size in `Reserved3` (0x40); the base name as a `UNICODE_STRING` at 0x58.

use crate::{Process, read_u64, write_bytes, write_u32, write_u64};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;

/// `PEB.Ldr`.
pub const PEB_LDR: u64 = 0x18;
/// List heads in `PEB_LDR_DATA`.
pub const LDR_IN_LOAD_ORDER: u64 = 0x10;
pub const LDR_IN_MEMORY_ORDER: u64 = 0x20;
pub const LDR_IN_INIT_ORDER: u64 = 0x30;
/// Bytes Poly allocates for `PEB_LDR_DATA` (the documented 0x30 and the
/// initialisation-order head).
pub const LDR_DATA_SIZE: usize = 0x40;

/// Fields of `LDR_DATA_TABLE_ENTRY`.
pub const ENTRY_IN_LOAD_ORDER: u64 = 0x00;
pub const ENTRY_IN_MEMORY_ORDER: u64 = 0x10;
pub const ENTRY_IN_INIT_ORDER: u64 = 0x20;
pub const ENTRY_DLL_BASE: u64 = 0x30;
pub const ENTRY_ENTRY_POINT: u64 = 0x38;
pub const ENTRY_SIZE_OF_IMAGE: u64 = 0x40;
pub const ENTRY_FULL_NAME: u64 = 0x48;
pub const ENTRY_BASE_NAME: u64 = 0x58;
pub const ENTRY_CHECKSUM: u64 = 0x78;
pub const ENTRY_TIME_DATE_STAMP: u64 = 0x80;
/// Bytes of an entry Poly allocates; the two names follow.
pub const ENTRY_SIZE: usize = 0x100;

/// Make `PEB_LDR_DATA` with three empty lists and point `PEB.Ldr` at it.
pub fn create(p: &mut dyn Process, peb: u64) -> Result<u64, NtStatus> {
    let ldr = p.alloc(LDR_DATA_SIZE)?;
    for head in [LDR_IN_LOAD_ORDER, LDR_IN_MEMORY_ORDER, LDR_IN_INIT_ORDER] {
        write_u64(p, ldr + head, ldr + head)?;
        write_u64(p, ldr + head + 8, ldr + head)?;
    }
    if peb != 0 {
        write_u64(p, peb + PEB_LDR, ldr)?;
    }
    Ok(ldr)
}

/// Insert `link` at the tail of the list headed by `head`.
pub fn insert_tail(p: &mut dyn Process, head: u64, link: u64) -> Result<(), NtStatus> {
    let last = read_u64(p, head + 8)?;
    write_u64(p, link, head)?;
    write_u64(p, link + 8, last)?;
    write_u64(p, last, link)?;
    write_u64(p, head + 8, link)
}

/// Take `link` out of its list (a link on no list points at itself).
pub fn remove(p: &mut dyn Process, link: u64) -> Result<(), NtStatus> {
    let next = read_u64(p, link)?;
    let prev = read_u64(p, link + 8)?;
    if next == 0 || prev == 0 {
        return Ok(());
    }
    write_u64(p, prev, next)?;
    write_u64(p, next + 8, prev)?;
    write_u64(p, link, link)?;
    write_u64(p, link + 8, link)
}

/// What an entry describes.
pub struct EntryData<'a> {
    pub base: u64,
    pub entry_point: u64,
    pub size: u64,
    pub full_name: &'a [u16],
    pub base_name: &'a [u16],
    pub checksum: u32,
    pub time_date_stamp: u32,
}

fn name_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().chain(&[0]).flat_map(|c| c.to_le_bytes()).collect()
}

/// Make an entry (names after it, NUL-terminated) with its links on no
/// list, and return its address.
pub fn create_entry(p: &mut dyn Process, d: &EntryData<'_>) -> Result<u64, NtStatus> {
    let full = name_bytes(d.full_name);
    let base = name_bytes(d.base_name);
    let e = p.alloc(ENTRY_SIZE + full.len() + base.len())?;
    for link in [ENTRY_IN_LOAD_ORDER, ENTRY_IN_MEMORY_ORDER, ENTRY_IN_INIT_ORDER] {
        write_u64(p, e + link, e + link)?;
        write_u64(p, e + link + 8, e + link)?;
    }
    write_u64(p, e + ENTRY_DLL_BASE, d.base)?;
    write_u64(p, e + ENTRY_ENTRY_POINT, d.entry_point)?;
    write_u64(p, e + ENTRY_SIZE_OF_IMAGE, d.size)?;
    let full_at = e + ENTRY_SIZE as u64;
    let base_at = full_at + full.len() as u64;
    for (field, at, units) in [(ENTRY_FULL_NAME, full_at, d.full_name), (ENTRY_BASE_NAME, base_at, d.base_name)] {
        let len = (units.len() * 2).min(0xFFFC) as u16;
        let mut s = [0u8; 16];
        s[0..2].copy_from_slice(&len.to_le_bytes());
        s[2..4].copy_from_slice(&(len + 2).to_le_bytes());
        s[8..16].copy_from_slice(&at.to_le_bytes());
        write_bytes(p, e + field, &s)?;
    }
    write_bytes(p, full_at, &full)?;
    write_bytes(p, base_at, &base)?;
    write_u32(p, e + ENTRY_CHECKSUM, d.checksum)?;
    write_u32(p, e + ENTRY_TIME_DATE_STAMP, d.time_date_stamp)?;
    Ok(e)
}

/// Unlink an entry from every list and free it.
pub fn destroy_entry(p: &mut dyn Process, e: u64) -> Result<(), NtStatus> {
    for link in [ENTRY_IN_LOAD_ORDER, ENTRY_IN_MEMORY_ORDER, ENTRY_IN_INIT_ORDER] {
        remove(p, e + link)?;
    }
    p.free_alloc(e);
    Ok(())
}

/// The entries of a list, in order: each link's address minus the link's
/// offset in the entry. Stops after `limit` steps (a corrupt list).
pub fn walk(p: &mut dyn Process, head: u64, link_offset: u64, limit: usize) -> Result<Vec<u64>, NtStatus> {
    let mut out = Vec::new();
    let mut at = read_u64(p, head)?;
    while at != head && out.len() < limit {
        out.push(at.wrapping_sub(link_offset));
        at = read_u64(p, at)?;
    }
    Ok(out)
}
