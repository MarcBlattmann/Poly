//! The tool help library's process snapshot (`tlhelp32.h`; sheets `kernel32.dll/CreateToolhelp32Snapshot`,
//! `Process32FirstW`, `Process32NextW`).
//!
//! # The snapshot
//!
//! A snapshot is a copy of the system's process list taken when it is made: the records of
//! `ntdll!PolyProcessList` (the NT personality's `proclist`), kept in a section of the paging
//! file whose handle is the snapshot handle. `CloseHandle` frees it, `TH32CS_INHERIT` makes the
//! handle inheritable, and the walk position lives in the section, so it belongs to the snapshot
//! (two threads walking one snapshot share it, as the sheet says). The section starts with a
//! [`HEADER_SIZE`]-byte header: [`MAGIC`] `u32`, the walk position `u32` (the index of the entry
//! `Process32NextW` gives next), the number of records `u32`, a reserved `u32`; the records follow.
//!
//! # A record
//!
//! [`RECORD_SIZE`] bytes: process id `u32` at 0, parent id `u32` at 4, thread count `u32` at 8,
//! base priority `i32` at 12, the image file name (no directory) as NUL-terminated UTF-16 in
//! [`NAME_UNITS`] units at 16.
//!
//! # `PROCESSENTRY32W`
//!
//! `tlhelp32.h`: `dwSize`, `cntUsage`, `th32ProcessID` (`DWORD`s), `th32DefaultHeapID`
//! (`ULONG_PTR`, aligned), `th32ModuleID`, `cntThreads`, `th32ParentProcessID`, `pcPriClassBase`
//! (`LONG`), `dwFlags`, `szExeFile[MAX_PATH]`, the size rounded up to the pointer alignment: 568
//! bytes in a 64-bit process (the sheet of `Process32FirstW` lists the offsets), 556 in a 32-bit
//! one. `dwSize` must be exactly that size (`ERROR_BAD_LENGTH` otherwise); `cntUsage`, the heap
//! and module ids and `dwFlags` are 0.

use crate::error::*;
use alloc::vec::Vec;

/// The units of a record's name field (`MAX_PATH`).
pub const NAME_UNITS: usize = 260;
/// The size of one record of `PolyProcessList`.
pub const RECORD_SIZE: usize = 16 + 2 * NAME_UNITS;
/// The snapshot section's header size.
pub const HEADER_SIZE: usize = 16;
/// The first `u32` of a snapshot section ("PTH1").
pub const MAGIC: u32 = u32::from_le_bytes(*b"PTH1");

/// `TH32CS_SNAPPROCESS`.
pub const TH32CS_SNAPPROCESS: u32 = 0x2;
/// `TH32CS_SNAPMODULE`, `TH32CS_SNAPMODULE32`.
pub const TH32CS_SNAPMODULE: u32 = 0x8;
pub const TH32CS_SNAPMODULE32: u32 = 0x10;
/// `TH32CS_INHERIT`.
pub const TH32CS_INHERIT: u32 = 0x8000_0000;

/// The layout of `PROCESSENTRY32W` for a pointer size of `ptr` bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryLayout {
    pub size: usize,
    pub process_id: usize,
    pub threads: usize,
    pub parent: usize,
    pub priority: usize,
    pub flags: usize,
    pub name: usize,
}

impl EntryLayout {
    pub const fn for_pointer(ptr: usize) -> EntryLayout {
        // dwSize, cntUsage, th32ProcessID, then the pointer-aligned heap id.
        let heap = (12 + ptr - 1) / ptr * ptr;
        let module = heap + ptr;
        let threads = module + 4;
        let name = threads + 16;
        let end = name + 2 * NAME_UNITS;
        EntryLayout {
            size: end.div_ceil(ptr) * ptr,
            process_id: 8,
            threads,
            parent: threads + 4,
            priority: threads + 8,
            flags: threads + 12,
            name,
        }
    }

    /// The layout of this build.
    pub const NATIVE: EntryLayout = EntryLayout::for_pointer(core::mem::size_of::<usize>());

    /// Fill `entry` (of [`Self::size`] bytes, `dwSize` already right) from `record`: every field
    /// after `dwSize` is written.
    pub fn fill(&self, entry: &mut [u8], record: &[u8]) {
        entry[4..self.size].fill(0);
        let put = |e: &mut [u8], at: usize, from: usize| e[at..at + 4].copy_from_slice(&record[from..from + 4]);
        put(entry, self.process_id, 0);
        put(entry, self.parent, 4);
        put(entry, self.threads, 8);
        put(entry, self.priority, 12);
        let name = &record[16..16 + 2 * NAME_UNITS];
        entry[self.name..self.name + 2 * NAME_UNITS].copy_from_slice(name);
        // The name always ends inside the field.
        entry[self.name + 2 * (NAME_UNITS - 1)..self.name + 2 * NAME_UNITS].fill(0);
    }
}

/// The section's contents for `records` (the list `PolyProcessList` gave, or nothing for a
/// snapshot without `TH32CS_SNAPPROCESS`).
pub fn snapshot_image(records: &[u8]) -> Vec<u8> {
    let count = records.len() / RECORD_SIZE;
    let mut out = Vec::with_capacity(HEADER_SIZE + count * RECORD_SIZE);
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(count as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&records[..count * RECORD_SIZE]);
    out
}

/// Whether the record list holds process `pid`.
pub fn lists(records: &[u8], pid: u32) -> bool {
    records.chunks_exact(RECORD_SIZE).any(|r| u32::from_le_bytes(r[0..4].try_into().unwrap()) == pid)
}

/// Which step of a walk: `Process32FirstW` (restart at the first entry) or `Process32NextW`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    First,
    Next,
}

/// One step of a walk over the snapshot `section` (header and records): the record to copy and
/// the new walk position, or `ERROR_NO_MORE_FILES` past the end (the position stays).
/// `ERROR_INVALID_HANDLE` when `section` is not a snapshot.
pub fn step(section: &[u8], step: Step) -> Result<(usize, u32), u32> {
    if section.len() < HEADER_SIZE || u32::from_le_bytes(section[0..4].try_into().unwrap()) != MAGIC {
        return Err(ERROR_INVALID_HANDLE);
    }
    let position = u32::from_le_bytes(section[4..8].try_into().unwrap());
    let count = u32::from_le_bytes(section[8..12].try_into().unwrap());
    let index = if step == Step::First { 0 } else { position };
    if index >= count || HEADER_SIZE + (index as usize + 1) * RECORD_SIZE > section.len() {
        return Err(ERROR_NO_MORE_FILES);
    }
    Ok((HEADER_SIZE + index as usize * RECORD_SIZE, index + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, parent: u32, name: &str) -> Vec<u8> {
        let mut r = alloc::vec![0u8; RECORD_SIZE];
        r[0..4].copy_from_slice(&pid.to_le_bytes());
        r[4..8].copy_from_slice(&parent.to_le_bytes());
        r[8..12].copy_from_slice(&2u32.to_le_bytes());
        r[12..16].copy_from_slice(&8i32.to_le_bytes());
        for (i, c) in name.encode_utf16().enumerate() {
            r[16 + 2 * i..18 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        r
    }

    #[test]
    fn entry_layouts_of_both_widths() {
        let x64 = EntryLayout::for_pointer(8);
        assert_eq!((x64.size, x64.threads, x64.parent, x64.priority, x64.flags, x64.name), (568, 28, 32, 36, 40, 44));
        let x86 = EntryLayout::for_pointer(4);
        assert_eq!((x86.size, x86.threads, x86.name), (556, 20, 36));
    }

    #[test]
    fn fill_writes_the_fields_and_zeroes_the_rest() {
        let l = EntryLayout::for_pointer(8);
        let mut e = alloc::vec![0xAAu8; l.size];
        e[0..4].copy_from_slice(&568u32.to_le_bytes());
        l.fill(&mut e, &record(0x400, 4, "setup.exe"));
        let u = |at: usize| u32::from_le_bytes(e[at..at + 4].try_into().unwrap());
        assert_eq!((u(0), u(4), u(8), u(16), u(24)), (568, 0, 0x400, 0, 0));
        assert_eq!((u(28), u(32), u(36), u(40)), (2, 4, 8, 0));
        assert_eq!(&e[44..46], &[b's', 0]);
        assert_eq!(&e[44 + 18..44 + 20], &[0, 0], "NUL after the name");
    }

    #[test]
    fn walks_restart_and_end() {
        let mut records = record(0, 0, "[System Process]");
        records.extend(record(4, 0, "System"));
        let mut s = snapshot_image(&records);
        assert_eq!(step(&s, Step::Next), Ok((HEADER_SIZE, 1)), "Next on a fresh snapshot gives the first");
        s[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(step(&s, Step::Next), Ok((HEADER_SIZE + RECORD_SIZE, 2)));
        s[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(step(&s, Step::Next), Err(ERROR_NO_MORE_FILES));
        assert_eq!(step(&s, Step::First), Ok((HEADER_SIZE, 1)));
        assert_eq!(step(&snapshot_image(&[]), Step::First), Err(ERROR_NO_MORE_FILES));
        assert_eq!(step(&[0u8; 16], Step::First), Err(ERROR_INVALID_HANDLE));
        assert!(lists(&records, 4) && !lists(&records, 8));
    }
}
