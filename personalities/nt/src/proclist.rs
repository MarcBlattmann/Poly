//! The system's process list for `kernelbase`'s tool help snapshot (`CreateToolhelp32Snapshot`,
//! `Process32FirstW`/`NextW`; sheets of `kernel32.dll`): Poly's own service
//! `PolyProcessList(Buffer, Length, ReturnLength*)`, in the manner of ADR 0206 (system information
//! from a system call of Poly's, not a layout of Windows' undocumented structures).
//!
//! # What is listed
//!
//! Every live NT process of the system (a process that has ended is not listed, even while a
//! handle to it is open, as the sheet of `CreateToolhelp32Snapshot` says), after two entries
//! that every Windows list starts with: pid 0 `[System Process]` and pid 4 `System` (sheet of
//! `Process32FirstW`). Poly has neither process; `System` stands for the kernel, so a process the
//! kernel started (parent 0 in [`crate::system::NtProcess::parent`]) is listed with parent 4.
//! NT process ids are multiples of 4 from 0x400 up ([`crate::ids`]), so they never meet 0 or 4.
//! Processes of the other personalities (Linux, native) are not NT processes and are not listed.
//!
//! # The records
//!
//! `ReturnLength` gets the size of the whole list; a `Length` smaller than that is
//! `STATUS_BUFFER_TOO_SMALL` with nothing written. The records are `poly_kernelbase::toolhelp`'s
//! (the reader's crate): process id, parent id, thread count, base priority, image file name.

use crate::abi;
use crate::kernel::NtKernel;
use crate::system::NtProcess;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_BUFFER_TOO_SMALL, STATUS_SUCCESS};

pub use poly_kernelbase::toolhelp::{NAME_UNITS, RECORD_SIZE};
/// The id the list gives the kernel (`System`).
pub const SYSTEM_PID: u32 = 4;

/// One process as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessSummary {
    pub pid: u32,
    /// The creator's id (0: the kernel started it).
    pub parent: u32,
    pub threads: u32,
    /// Its priority class as `ProcessPriorityClass` numbers it (1 idle ... 6 above normal).
    pub priority_class: u8,
    /// The image file's name without its directory.
    pub name: Vec<u16>,
}

impl ProcessSummary {
    /// The summary of a live process.
    pub fn of(p: &NtProcess) -> ProcessSummary {
        let name = p.image_name.as_ref().map_or(&[][..], |(_, path)| &path[..]);
        let start = name.iter().rposition(|&c| c == u16::from(b'\\') || c == u16::from(b'/')).map_or(0, |i| i + 1);
        ProcessSummary {
            pid: p.pid.0,
            parent: p.parent,
            threads: p.threads.len() as u32,
            priority_class: p.priority_class,
            name: name[start..].to_vec(),
        }
    }
}

fn record(out: &mut Vec<u8>, pid: u32, parent: u32, threads: u32, priority: i32, name: &[u16]) {
    let at = out.len();
    out.resize(at + RECORD_SIZE, 0);
    out[at..at + 4].copy_from_slice(&pid.to_le_bytes());
    out[at + 4..at + 8].copy_from_slice(&parent.to_le_bytes());
    out[at + 8..at + 12].copy_from_slice(&threads.to_le_bytes());
    out[at + 12..at + 16].copy_from_slice(&priority.to_le_bytes());
    for (i, &c) in name.iter().take(NAME_UNITS - 1).enumerate() {
        out[at + 16 + 2 * i..at + 18 + 2 * i].copy_from_slice(&c.to_le_bytes());
    }
}

/// The whole list as records: `[System Process]`, `System`, then `processes` by id.
pub fn records(processes: &[ProcessSummary]) -> Vec<u8> {
    let mut sorted: Vec<&ProcessSummary> = processes.iter().filter(|p| p.pid != 0 && p.pid != SYSTEM_PID).collect();
    sorted.sort_by_key(|p| p.pid);
    sorted.dedup_by_key(|p| p.pid);
    let mut out = Vec::with_capacity((sorted.len() + 2) * RECORD_SIZE);
    let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    record(&mut out, 0, 0, 1, 0, &w("[System Process]"));
    record(&mut out, SYSTEM_PID, 0, 1, 8, &w("System"));
    for p in sorted {
        let parent = if p.parent == 0 { SYSTEM_PID } else { p.parent };
        record(
            &mut out,
            p.pid,
            parent,
            p.threads.max(1),
            crate::threads::class_base_priority(p.priority_class),
            &p.name,
        );
    }
    out
}

/// `PolyProcessList(Buffer, Length, ReturnLength*)`.
pub fn process_list<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus, NtStatus> {
    let [buf, length, ret] = abi::args::<K, 3>(k, regs)?;
    let list = records(&k.processes());
    abi::write_opt_u32(k, ret, list.len() as u32)?;
    if (length as u32 as usize) < list.len() {
        return Err(STATUS_BUFFER_TOO_SMALL);
    }
    abi::write(k, buf, &list)?;
    Ok(STATUS_SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::Service;
    use crate::tests::Fake;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn name_at(list: &[u8], i: usize) -> alloc::string::String {
        let at = i * RECORD_SIZE + 16;
        let units: Vec<u16> = list[at..at + 2 * NAME_UNITS]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        alloc::string::String::from_utf16(&units).unwrap()
    }

    fn u32_at(list: &[u8], i: usize, field: usize) -> u32 {
        let at = i * RECORD_SIZE + field;
        u32::from_le_bytes(list[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn the_list_starts_with_the_system_entries_and_is_sorted() {
        let p = |pid, parent, class, name: &str| ProcessSummary {
            pid,
            parent,
            threads: 3,
            priority_class: class,
            name: w(name),
        };
        let list = records(&[p(0x408, 0x400, 2, "child.exe"), p(0x400, 0, 3, "setup.exe")]);
        assert_eq!(list.len(), 4 * RECORD_SIZE);
        assert_eq!((u32_at(&list, 0, 0), name_at(&list, 0)), (0, "[System Process]".into()));
        assert_eq!((u32_at(&list, 1, 0), name_at(&list, 1)), (4, "System".into()));
        assert_eq!((u32_at(&list, 2, 0), u32_at(&list, 2, 4), name_at(&list, 2)), (0x400, 4, "setup.exe".into()));
        assert_eq!(u32_at(&list, 2, 12), 13, "high priority class");
        assert_eq!((u32_at(&list, 3, 0), u32_at(&list, 3, 4), u32_at(&list, 3, 8)), (0x408, 0x400, 3));
        assert_eq!(u32_at(&list, 3, 12), 8);
    }

    #[test]
    fn a_long_name_is_cut_and_ends_with_a_nul() {
        let long: alloc::string::String = core::iter::repeat_n('a', 400).collect();
        let list = records(&[ProcessSummary { pid: 0x400, parent: 0, threads: 1, priority_class: 2, name: w(&long) }]);
        assert_eq!(name_at(&list, 2).len(), NAME_UNITS - 1);
    }

    #[test]
    fn the_service_sizes_and_fills_the_buffer() {
        let mut f = Fake::new();
        let (buf, ret) = (f.alloc(8 * RECORD_SIZE), f.alloc(4));
        assert_eq!(f.call(Service::PolyProcessList, &[buf, 10, ret]), STATUS_BUFFER_TOO_SMALL);
        let need = f.u32_at(ret) as u64;
        assert!(need >= 3 * RECORD_SIZE as u64, "the two system entries and the caller");
        assert_eq!(f.call(Service::PolyProcessList, &[buf, need, ret]), STATUS_SUCCESS);
        assert_eq!(f.u32_at(buf), 0);
        assert_eq!(f.u32_at(buf + RECORD_SIZE as u64), SYSTEM_PID);
    }
}
