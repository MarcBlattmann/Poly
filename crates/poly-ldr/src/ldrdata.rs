//! The loader's lists in process memory: `PEB.Ldr` (`PEB_LDR_DATA`) and one
//! `LDR_DATA_TABLE_ENTRY` per module, linked by `LIST_ENTRY`s (`winternl.h`,
//! `ntdef.h`; offsets for x64 in the constants below, for a 32-bit process in
//! [`LdrLayout::X86`]).
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
//!
//! For a 32-bit process ([`Bitness::X86`]) the same structures have 4-byte
//! pointers (`winternl.h`, 32-bit): `PEB.Ldr` 0x0C; in `PEB_LDR_DATA`
//! `Length` 0x00, `Initialized` 0x04, the in-load-order head 0x0C (Poly's
//! use of `Reserved2`), `InMemoryOrderModuleList` 0x14, the in-init-order
//! head 0x1C, 0x30 bytes in all; in each entry the in-load-order links 0x00
//! (`Reserved1`), `InMemoryOrderLinks` 0x08, the in-init-order links 0x10
//! (`Reserved2`), `DllBase` 0x18, `EntryPoint` 0x1C, the image size 0x20
//! (`Reserved3`), `FullDllName` 0x24, the base name 0x2C (Poly's use of
//! `Reserved4`), `CheckSum` 0x40 and `TimeDateStamp` 0x44. A `LIST_ENTRY` is
//! two pointers; a `UNICODE_STRING` is `Length` (2), `MaximumLength` (2) and
//! the buffer pointer, 8 bytes.

use crate::status::STATUS_INVALID_PARAMETER;
use crate::{Process, read_ptr, write_bytes, write_ptr, write_u32};
use alloc::vec;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;

/// `PEB.Ldr`.
pub const PEB_LDR: u64 = 0x18;
/// List heads in `PEB_LDR_DATA`.
pub const LDR_IN_LOAD_ORDER: u64 = 0x10;
pub const LDR_IN_MEMORY_ORDER: u64 = 0x20;
pub const LDR_IN_INIT_ORDER: u64 = 0x30;
/// `sizeof(PEB_LDR_DATA)` on x64 (`Length`, `Initialized`, `SsHandle`, the three list heads,
/// `EntryInProgress`, `ShutdownInProgress`, `ShutdownThreadId`): programs that walk the lists
/// check `Length` and `Initialized`.
pub const LDR_DATA_SIZE: usize = 0x58;
/// `PEB_LDR_DATA.Length` and `.Initialized` (a `BOOLEAN`).
pub const LDR_LENGTH: u64 = 0x00;
pub const LDR_INITIALIZED: u64 = 0x04;

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

/// The width of the process's pointers: which images it runs and how the
/// loader lays out its structures. Chosen when the loader is made
/// ([`crate::Config::bitness`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bitness {
    /// A 64-bit process: AMD64 PE32+ images, 8-byte pointers.
    #[default]
    X64,
    /// A 32-bit process: I386 PE32 images, 4-byte pointers.
    X86,
}

impl Bitness {
    /// The structure offsets of a process of this width.
    pub const fn layout(self) -> &'static LdrLayout {
        match self {
            Bitness::X64 => &LdrLayout::X64,
            Bitness::X86 => &LdrLayout::X86,
        }
    }

    /// Bytes in a pointer: 8 or 4.
    pub const fn pointer_size(self) -> u64 {
        self.layout().ptr
    }
}

/// Where the loader's structures keep their fields in a process of one
/// [`Bitness`] (see the module documentation). Offsets are bytes from the
/// start of the structure; `teb_tls_pointer` is from the start of the TEB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LdrLayout {
    pub bitness: Bitness,
    /// Bytes in a pointer (and in each `LIST_ENTRY` link).
    pub ptr: u64,
    /// `PEB.Ldr`.
    pub peb_ldr: u64,
    /// `TEB.ThreadLocalStoragePointer`.
    pub teb_tls_pointer: u64,
    /// `sizeof(PEB_LDR_DATA)`, which is also its `Length`.
    pub ldr_size: usize,
    pub ldr_length: u64,
    pub ldr_initialized: u64,
    /// The list heads in `PEB_LDR_DATA`.
    pub ldr_in_load_order: u64,
    pub ldr_in_memory_order: u64,
    pub ldr_in_init_order: u64,
    /// Fields of `LDR_DATA_TABLE_ENTRY`.
    pub entry_in_load_order: u64,
    pub entry_in_memory_order: u64,
    pub entry_in_init_order: u64,
    pub entry_dll_base: u64,
    pub entry_entry_point: u64,
    pub entry_size_of_image: u64,
    pub entry_full_name: u64,
    pub entry_base_name: u64,
    pub entry_checksum: u64,
    pub entry_time_date_stamp: u64,
    /// Bytes of an entry Poly allocates; the two names follow.
    pub entry_size: usize,
}

impl LdrLayout {
    /// The x64 layout (the constants of this module).
    pub const X64: LdrLayout = LdrLayout {
        bitness: Bitness::X64,
        ptr: 8,
        peb_ldr: PEB_LDR,
        teb_tls_pointer: 0x58,
        ldr_size: LDR_DATA_SIZE,
        ldr_length: LDR_LENGTH,
        ldr_initialized: LDR_INITIALIZED,
        ldr_in_load_order: LDR_IN_LOAD_ORDER,
        ldr_in_memory_order: LDR_IN_MEMORY_ORDER,
        ldr_in_init_order: LDR_IN_INIT_ORDER,
        entry_in_load_order: ENTRY_IN_LOAD_ORDER,
        entry_in_memory_order: ENTRY_IN_MEMORY_ORDER,
        entry_in_init_order: ENTRY_IN_INIT_ORDER,
        entry_dll_base: ENTRY_DLL_BASE,
        entry_entry_point: ENTRY_ENTRY_POINT,
        entry_size_of_image: ENTRY_SIZE_OF_IMAGE,
        entry_full_name: ENTRY_FULL_NAME,
        entry_base_name: ENTRY_BASE_NAME,
        entry_checksum: ENTRY_CHECKSUM,
        entry_time_date_stamp: ENTRY_TIME_DATE_STAMP,
        entry_size: ENTRY_SIZE,
    };

    /// The layout of a 32-bit process (see the module documentation).
    /// `TEB.ThreadLocalStoragePointer` is at 0x2C (the PE specification's
    /// `FS:[0x2C]`).
    pub const X86: LdrLayout = LdrLayout {
        bitness: Bitness::X86,
        ptr: 4,
        peb_ldr: 0x0C,
        teb_tls_pointer: 0x2C,
        ldr_size: 0x30,
        ldr_length: 0x00,
        ldr_initialized: 0x04,
        ldr_in_load_order: 0x0C,
        ldr_in_memory_order: 0x14,
        ldr_in_init_order: 0x1C,
        entry_in_load_order: 0x00,
        entry_in_memory_order: 0x08,
        entry_in_init_order: 0x10,
        entry_dll_base: 0x18,
        entry_entry_point: 0x1C,
        entry_size_of_image: 0x20,
        entry_full_name: 0x24,
        entry_base_name: 0x2C,
        entry_checksum: 0x40,
        entry_time_date_stamp: 0x44,
        entry_size: 0x80,
    };

    /// Bytes in a `LIST_ENTRY`.
    pub const fn list_entry_size(&self) -> u64 {
        2 * self.ptr
    }

    /// Bytes in a `UNICODE_STRING`: `Length`, `MaximumLength`, then the
    /// buffer pointer at offset [`ptr`](Self::ptr) (after padding on x64).
    pub const fn unicode_string_size(&self) -> usize {
        2 * self.ptr as usize
    }
}

/// Make `PEB_LDR_DATA` with three empty lists and point `PEB.Ldr` at it.
pub fn create(p: &mut dyn Process, lay: &LdrLayout, peb: u64) -> Result<u64, NtStatus> {
    let w = lay.ptr;
    let ldr = p.alloc(lay.ldr_size)?;
    write_bytes(p, ldr, &vec![0u8; lay.ldr_size])?;
    write_u32(p, ldr + lay.ldr_length, lay.ldr_size as u32)?;
    write_bytes(p, ldr + lay.ldr_initialized, &[1])?;
    for head in [lay.ldr_in_load_order, lay.ldr_in_memory_order, lay.ldr_in_init_order] {
        write_ptr(p, w, ldr + head, ldr + head)?;
        write_ptr(p, w, ldr + head + w, ldr + head)?;
    }
    if peb != 0 {
        write_ptr(p, w, peb + lay.peb_ldr, ldr)?;
    }
    Ok(ldr)
}

/// Insert `link` at the tail of the list headed by `head`.
pub fn insert_tail(p: &mut dyn Process, lay: &LdrLayout, head: u64, link: u64) -> Result<(), NtStatus> {
    let w = lay.ptr;
    let last = read_ptr(p, w, head + w)?;
    write_ptr(p, w, link, head)?;
    write_ptr(p, w, link + w, last)?;
    write_ptr(p, w, last, link)?;
    write_ptr(p, w, head + w, link)
}

/// Take `link` out of its list (a link on no list points at itself).
pub fn remove(p: &mut dyn Process, lay: &LdrLayout, link: u64) -> Result<(), NtStatus> {
    let w = lay.ptr;
    let next = read_ptr(p, w, link)?;
    let prev = read_ptr(p, w, link + w)?;
    if next == 0 || prev == 0 {
        return Ok(());
    }
    write_ptr(p, w, prev, next)?;
    write_ptr(p, w, next + w, prev)?;
    write_ptr(p, w, link, link)?;
    write_ptr(p, w, link + w, link)
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
pub fn create_entry(p: &mut dyn Process, lay: &LdrLayout, d: &EntryData<'_>) -> Result<u64, NtStatus> {
    let w = lay.ptr;
    let full = name_bytes(d.full_name);
    let base = name_bytes(d.base_name);
    let e = p.alloc(lay.entry_size + full.len() + base.len())?;
    for link in [lay.entry_in_load_order, lay.entry_in_memory_order, lay.entry_in_init_order] {
        write_ptr(p, w, e + link, e + link)?;
        write_ptr(p, w, e + link + w, e + link)?;
    }
    write_ptr(p, w, e + lay.entry_dll_base, d.base)?;
    write_ptr(p, w, e + lay.entry_entry_point, d.entry_point)?;
    write_ptr(p, w, e + lay.entry_size_of_image, d.size)?;
    let full_at = e + lay.entry_size as u64;
    let base_at = full_at + full.len() as u64;
    for (field, at, units) in [(lay.entry_full_name, full_at, d.full_name), (lay.entry_base_name, base_at, d.base_name)]
    {
        let len = (units.len() * 2).min(0xFFFC) as u16;
        let mut s = vec![0u8; lay.unicode_string_size()];
        s[0..2].copy_from_slice(&len.to_le_bytes());
        s[2..4].copy_from_slice(&(len + 2).to_le_bytes());
        let at = at.to_le_bytes();
        if w == 4 {
            // The buffer pointer is 32 bits: the name must lie below 4 GiB.
            if at[4..].iter().any(|&b| b != 0) {
                return Err(STATUS_INVALID_PARAMETER);
            }
            s[4..8].copy_from_slice(&at[..4]);
        } else {
            s[8..16].copy_from_slice(&at);
        }
        write_bytes(p, e + field, &s)?;
    }
    write_bytes(p, full_at, &full)?;
    write_bytes(p, base_at, &base)?;
    write_u32(p, e + lay.entry_checksum, d.checksum)?;
    write_u32(p, e + lay.entry_time_date_stamp, d.time_date_stamp)?;
    Ok(e)
}

/// Unlink an entry from every list and free it.
pub fn destroy_entry(p: &mut dyn Process, lay: &LdrLayout, e: u64) -> Result<(), NtStatus> {
    for link in [lay.entry_in_load_order, lay.entry_in_memory_order, lay.entry_in_init_order] {
        remove(p, lay, e + link)?;
    }
    p.free_alloc(e);
    Ok(())
}

/// The entries of a list of a 64-bit process, in order: each link's address
/// minus the link's offset in the entry. Stops after `limit` steps (a
/// corrupt list).
pub fn walk(p: &mut dyn Process, head: u64, link_offset: u64, limit: usize) -> Result<Vec<u64>, NtStatus> {
    walk_in(p, Bitness::X64, head, link_offset, limit)
}

/// [`walk`] for a process of the given [`Bitness`].
pub fn walk_in(
    p: &mut dyn Process,
    bitness: Bitness,
    head: u64,
    link_offset: u64,
    limit: usize,
) -> Result<Vec<u64>, NtStatus> {
    let w = bitness.pointer_size();
    let mut out = Vec::new();
    let mut at = read_ptr(p, w, head)?;
    while at != head && out.len() < limit {
        out.push(at.wrapping_sub(link_offset));
        at = read_ptr(p, w, at)?;
    }
    Ok(out)
}
