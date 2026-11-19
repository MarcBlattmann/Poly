//! Change notification of directories: `NtNotifyChangeDirectoryFile(FileHandle, Event,
//! ApcRoutine, ApcContext, IoStatusBlock*, Buffer, Length, CompletionFilter, WatchTree)` (Learn:
//! "ZwNotifyChangeDirectoryFile"; `kernel32!ReadDirectoryChangesW`, sheet
//! `kernel32.dll/ReadDirectoryChangesW`).
//!
//! The first call on a directory's file object starts a watch: its filter and subtree flag stay
//! with the file object. Every change Poly's file services make (a name made, removed or renamed,
//! data written, attributes or times set) that the watch's filter covers becomes a
//! `FILE_NOTIFY_INFORMATION` record: the request waiting on the watch completes with the records,
//! or, while no request waits, they are kept for the next call, which then completes at once.
//! Records that do not fit the request's buffer complete it with no data and
//! `STATUS_NOTIFY_ENUM_DIR` (the caller rescans); so does a watch that kept more than
//! [`MAX_KEPT`] bytes of them. Closing the file object ends the watch (its request is cancelled
//! with the other requests of the file object).
//!
//! Only changes made through the NT file services are seen: a Linux or native process writing the
//! same directory does not notify.

use super::IoState;
use super::irp::{Irp, IrpOp};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_INVALID_PARAMETER, STATUS_SUCCESS};

/// `FILE_ACTION_*` (`winnt.h`).
pub const FILE_ACTION_ADDED: u32 = 1;
pub const FILE_ACTION_REMOVED: u32 = 2;
pub const FILE_ACTION_MODIFIED: u32 = 3;
pub const FILE_ACTION_RENAMED_OLD_NAME: u32 = 4;
pub const FILE_ACTION_RENAMED_NEW_NAME: u32 = 5;

/// `FILE_NOTIFY_CHANGE_*` (`winnt.h`).
pub const CHANGE_FILE_NAME: u32 = 0x1;
pub const CHANGE_DIR_NAME: u32 = 0x2;
pub const CHANGE_ATTRIBUTES: u32 = 0x4;
pub const CHANGE_SIZE: u32 = 0x8;
pub const CHANGE_LAST_WRITE: u32 = 0x10;
pub const CHANGE_LAST_ACCESS: u32 = 0x20;
pub const CHANGE_CREATION: u32 = 0x40;
/// Every bit a filter may have (`FILE_NOTIFY_VALID_MASK`, `ntifs.h`).
pub const VALID_FILTER: u32 = 0xFFF;

/// `STATUS_NOTIFY_ENUM_DIR` (`ntstatus.h`): the changes did not fit; enumerate the directory.
pub const STATUS_NOTIFY_ENUM_DIR: NtStatus = NtStatus(0x0000_010C);

/// The most record bytes a watch keeps while no request waits; past it the next request reports
/// `STATUS_NOTIFY_ENUM_DIR`.
pub const MAX_KEPT: usize = 64 * 1024;

/// One directory watch (a file object that asked for change notification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirWatch {
    pub volume: usize,
    /// The directory's path below the volume root (`\a\b`, `\` for the root).
    pub path: Vec<u16>,
    pub filter: u32,
    pub subtree: bool,
    /// Changes not yet reported: action and name relative to the directory.
    pub kept: Vec<(u32, Vec<u16>)>,
    /// More changes came than are kept.
    pub overflow: bool,
    /// The request waiting for changes, its buffer and length.
    pub waiting: Option<(u64, u64, usize)>,
}

/// What became of a notification request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requested {
    /// It waits for a change: its id.
    Pending(u64),
    /// It completed at once with this status (its records go out with the completion).
    Done(NtStatus),
}

/// What kind of change happened, for the filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A name made, removed or renamed (`directory`: the entry is a directory).
    Name { action: u32, directory: bool },
    /// Something about the entry changed: the filter bits that cover it.
    Modified { mask: u32 },
}

impl Change {
    fn action_and_mask(self) -> (u32, u32) {
        match self {
            Change::Name { action, directory } => {
                (action, if directory { CHANGE_DIR_NAME } else { CHANGE_FILE_NAME })
            }
            Change::Modified { mask } => (FILE_ACTION_MODIFIED, mask),
        }
    }
}

fn upcase(c: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c }
}

fn eq_fold(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| upcase(x) == upcase(y))
}

/// `path` without trailing separators (the root stays empty).
fn trim(path: &[u16]) -> &[u16] {
    let mut end = path.len();
    while end > 0 && path[end - 1] == u16::from(b'\\') {
        end -= 1;
    }
    &path[..end]
}

/// The name of `changed` (a path below the volume root) relative to the watched directory `dir`:
/// the last component for an entry of `dir`, a `sub\name` path for one below it when `subtree`;
/// `None` when the watch does not cover it.
pub fn relative_name(dir: &[u16], subtree: bool, changed: &[u16]) -> Option<Vec<u16>> {
    let (dir, changed) = (trim(dir), trim(changed));
    if changed.len() <= dir.len() + 1 || !eq_fold(&changed[..dir.len()], dir) || changed[dir.len()] != u16::from(b'\\')
    {
        return None;
    }
    let rest = &changed[dir.len() + 1..];
    if !subtree && rest.contains(&u16::from(b'\\')) {
        return None;
    }
    Some(rest.to_vec())
}

/// The `FILE_NOTIFY_INFORMATION` records for `changes`: `NextEntryOffset`, `Action`,
/// `FileNameLength` (bytes), the name; each record starts 4-byte aligned, the last has offset 0.
pub fn records(changes: &[(u32, Vec<u16>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut last: Option<usize> = None;
    for (action, name) in changes {
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        let at = out.len();
        if let Some(prev) = last {
            out[prev..prev + 4].copy_from_slice(&((at - prev) as u32).to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&action.to_le_bytes());
        out.extend_from_slice(&((2 * name.len()) as u32).to_le_bytes());
        for c in name {
            out.extend_from_slice(&c.to_le_bytes());
        }
        last = Some(at);
    }
    out
}

impl IoState {
    /// Start (or find) the watch of file object `cookie` on directory `path` of `volume`, and make
    /// `irp` (an [`IrpOp::Notify`]) its waiting request, or complete it at once with what the watch
    /// kept.
    pub fn notify_request(
        &mut self,
        cookie: u64,
        volume: usize,
        path: &[u16],
        filter: u32,
        subtree: bool,
        mut irp: Irp,
    ) -> Requested {
        let IrpOp::Notify { buf, len } = irp.op else { return Requested::Done(STATUS_INVALID_PARAMETER) };
        let w = self.watches.entry(cookie).or_insert_with(|| DirWatch {
            volume,
            path: path.to_vec(),
            filter,
            subtree,
            kept: Vec::new(),
            overflow: false,
            waiting: None,
        });
        if w.overflow || !w.kept.is_empty() {
            let (status, data) = take_kept(w, len);
            let n = data.len() as u64;
            let data = (n > 0).then_some((buf, data));
            // Done in the calling thread: nobody waits for it.
            irp.sync = false;
            self.complete_now(irp, status, n, data);
            return Requested::Done(status);
        }
        let id = irp.id;
        w.waiting = Some((id, buf, len));
        self.park(irp);
        Requested::Pending(id)
    }

    /// A change Poly's file services made to `path` (below the root of `volume`): every watch that
    /// covers it gets the record; a waiting request completes.
    pub fn note_change(&mut self, volume: usize, path: &[u16], change: Change) {
        self.note_changes(volume, &[(path, change)]);
    }

    /// Changes made together (a rename's old and new name): a waiting request gets all of them
    /// that its watch covers in one completion, in this order.
    pub fn note_changes(&mut self, volume: usize, changes: &[(&[u16], Change)]) {
        if self.watches.is_empty() {
            return;
        }
        let files = &self.files;
        self.watches.retain(|cookie, _| files.contains_key(cookie));
        let mut done: Vec<(u64, NtStatus, Option<(u64, Vec<u8>)>, u64)> = Vec::new();
        let irps = &self.irps;
        for w in self.watches.values_mut() {
            // A request that was cancelled meanwhile waits no more.
            if w.waiting.is_some_and(|(id, _, _)| !irps.contains_key(&id)) {
                w.waiting = None;
            }
            let mut matched = false;
            for &(path, change) in changes {
                let (action, mask) = change.action_and_mask();
                if w.volume != volume || w.filter & mask == 0 {
                    continue;
                }
                let Some(name) = relative_name(&w.path, w.subtree, path) else { continue };
                matched = true;
                if !w.overflow {
                    w.kept.push((action, name));
                    if records(&w.kept).len() > MAX_KEPT {
                        w.kept.clear();
                        w.overflow = true;
                    }
                }
            }
            if matched && let Some((id, buf, len)) = w.waiting.take() {
                let (status, data) = take_kept(w, len);
                let n = data.len() as u64;
                done.push((id, status, (n > 0).then_some((buf, data)), n));
            }
        }
        for (id, status, data, n) in done {
            self.complete(id, status, n, data);
        }
    }

    /// [`Self::note_change`] for the entry file object `cookie` is open on.
    pub fn note_file_change(&mut self, cookie: u64, change: Change) {
        if self.watches.is_empty() {
            return;
        }
        let Some(d) = self.files.get(&cookie).and_then(|f| f.disk()) else { return };
        if d.stream.is_some() {
            return;
        }
        let (volume, path) = (d.volume, d.path.clone());
        self.note_change(volume, &path, change);
    }
}

/// `NtNotifyChangeDirectoryFile(FileHandle, Event, ApcRoutine, ApcContext, IoStatusBlock*,
/// Buffer, Length, CompletionFilter, WatchTree)`: the handle needs `FILE_LIST_DIRECTORY` and must
/// be a directory's (`STATUS_INVALID_PARAMETER` otherwise), the filter must have a bit and only
/// valid ones (`STATUS_INVALID_PARAMETER`). `STATUS_PENDING` while the request waits; on a
/// synchronous file object the call waits for it.
pub fn notify_change_directory_file<K: crate::kernel::NtKernel>(
    k: &mut K,
    regs: &poly_abi::SyscallRegs,
) -> Result<NtStatus, NtStatus> {
    use super::consts::FILE_LIST_DIRECTORY;
    use super::services::{Notify, file_handle, wait_sync};
    let [h, event, apc_routine, apc_context, iosb, buf, len, filter, subtree] =
        crate::abi::args::<K, 9>(k, regs)?;
    let (len, filter, subtree) = (len as u32 as usize, filter as u32, subtree as u8 != 0);
    let n = Notify { event, apc_routine, apc_context, iosb };
    let caller = k.caller();
    let r = k.with_state(|sys, _, _| -> Result<(Requested, poly_ntobj::ObjectId, bool), NtStatus> {
        let file = file_handle(sys, &caller, poly_ntobj::Handle(h), FILE_LIST_DIRECTORY)?;
        n.check_event(sys, &caller)?;
        let rec = sys.io.file(file.1).ok_or(STATUS_INVALID_PARAMETER)?;
        let d = rec.disk().filter(|_| rec.kind == poly_vfs::FileType::Directory).ok_or(STATUS_INVALID_PARAMETER)?;
        if filter == 0 || filter & !VALID_FILTER != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let (volume, path, sync) = (d.volume, d.path.clone(), rec.is_sync());
        let irp = n.irp(sys, &caller, file, sync, IrpOp::Notify { buf, len })?;
        Ok((sys.io.notify_request(file.1, volume, &path, filter, subtree, irp), file.0, sync))
    })?;
    match r {
        (Requested::Done(status), _, _) => Ok(status),
        (Requested::Pending(irp), object, true) => Ok(wait_sync(k, object, irp).0),
        (Requested::Pending(_), _, false) => Ok(poly_ntobj::status::STATUS_PENDING),
    }
}

/// What the watch reports to a request with a buffer of `len` bytes: its records, or no data and
/// `STATUS_NOTIFY_ENUM_DIR` when they do not fit (or were too many to keep). The watch is empty
/// afterwards.
fn take_kept(w: &mut DirWatch, len: usize) -> (NtStatus, Vec<u8>) {
    let data = records(&w.kept);
    w.kept.clear();
    let overflow = core::mem::take(&mut w.overflow);
    if overflow || data.len() > len { (STATUS_NOTIFY_ENUM_DIR, Vec::new()) } else { (STATUS_SUCCESS, data) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn names_relative_to_the_watched_directory() {
        assert_eq!(relative_name(&w("\\t\\d"), false, &w("\\t\\d\\a.txt")), Some(w("a.txt")));
        assert_eq!(relative_name(&w("\\T\\D"), false, &w("\\t\\d\\a.txt")), Some(w("a.txt")), "without case");
        assert_eq!(relative_name(&w("\\t\\d"), false, &w("\\t\\d\\s\\a.txt")), None, "not the subtree");
        assert_eq!(relative_name(&w("\\t\\d"), true, &w("\\t\\d\\s\\a.txt")), Some(w("s\\a.txt")));
        assert_eq!(relative_name(&w("\\t\\d"), true, &w("\\t\\dx\\a.txt")), None, "another directory");
        assert_eq!(relative_name(&w("\\t\\d"), true, &w("\\t\\d")), None, "the directory itself");
        assert_eq!(relative_name(&w("\\"), false, &w("\\a.txt")), Some(w("a.txt")), "the root");
    }

    #[test]
    fn records_are_aligned_and_chained() {
        let r = records(&[(1, w("abc")), (2, w("de"))]);
        // 12 + 6 = 18, the next starts at 20.
        assert_eq!(u32::from_le_bytes(r[0..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(r[4..8].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(r[8..12].try_into().unwrap()), 6);
        assert_eq!(u32::from_le_bytes(r[20..24].try_into().unwrap()), 0, "the last record");
        assert_eq!(u32::from_le_bytes(r[24..28].try_into().unwrap()), 2);
        assert_eq!(r.len(), 20 + 12 + 4);
        // The sheet's overflow record: 12 + 2 * 28 = 68 bytes.
        assert_eq!(records(&[(1, w("rd_long_name_to_overflow.txt"))]).len(), 68);
    }

    #[test]
    fn kept_changes_that_do_not_fit_are_an_overflow() {
        let mut watch = DirWatch {
            volume: 0,
            path: w("\\d"),
            filter: CHANGE_FILE_NAME,
            subtree: false,
            kept: alloc::vec![(1, w("rd_long_name_to_overflow.txt"))],
            overflow: false,
            waiting: None,
        };
        assert_eq!(take_kept(&mut watch, 40), (STATUS_NOTIFY_ENUM_DIR, Vec::new()));
        assert!(watch.kept.is_empty());
        watch.kept.push((2, w("a")));
        let (status, data) = take_kept(&mut watch, 4096);
        assert_eq!((status, data.len()), (STATUS_SUCCESS, 14));
    }
}
