//! Directory enumeration: `NtQueryDirectoryFile`'s scan of a directory file
//! object (Learn: "ZwQueryDirectoryFile", "FILE_INFORMATION_CLASS").
//!
//! - The first call on a handle starts a scan: it takes the directory's
//!   names (a snapshot, so entries made or removed meanwhile do not disturb
//!   it) and the `FileName` expression, kept for the whole scan
//!   ("the FileName ... is used only on the first call; subsequent calls
//!   ignore it"). `RestartScan` starts again from the first entry, with a
//!   new expression when one is given (to be confirmed by the spec agent).
//! - Entries are matched without case ([`names::matches`]) and returned in
//!   NTFS's order, by their names in capitals compared unit by unit
//!   ([`ntfs_order`]; the specs' listings, e.g. `fo_cases`, show it), each aligned to 8 bytes with
//!   `NextEntryOffset` linking them (0 in the last). `ReturnSingleEntry`
//!   returns one.
//! - A first scan that matches nothing is `STATUS_NO_SUCH_FILE`; a later
//!   call with nothing left is `STATUS_NO_MORE_FILES`.
//! - A buffer smaller than one entry's fixed part is
//!   `STATUS_INFO_LENGTH_MISMATCH`. If the first entry's name does not fit,
//!   the fixed part and as much of the name as fits are returned with
//!   `STATUS_BUFFER_OVERFLOW`, and the entry is consumed (to be confirmed by
//!   the spec agent).
//! - `.` and `..` are listed for every directory but a volume's root, as
//!   NTFS does.
//! - A name that vanished since the snapshot is skipped.

use super::info::{self, DirEntryInfo};
use super::names::{self, vfs_to_wide};
use super::{IoState, consts::*, status_of};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_BUFFER_OVERFLOW, STATUS_INFO_LENGTH_MISMATCH, STATUS_INVALID_INFO_CLASS};
use poly_vfs::{At, NodeRef};

/// A directory handle's scan.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub pattern: Vec<u16>,
    pub names: Vec<Vec<u8>>,
    pub next: usize,
    /// Whether any entry has been returned.
    pub returned: bool,
}

/// One call's answer: the bytes for the caller's buffer and the status.
#[derive(Debug, PartialEq, Eq)]
pub struct Answer {
    pub bytes: Vec<u8>,
    pub status: NtStatus,
}

/// What a query asks for.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    pub class: u32,
    pub length: usize,
    pub single: bool,
    pub pattern: Option<&'a [u16]>,
    pub restart: bool,
}

/// A name's UTF-16 units in capitals (each unit on its own, as NTFS's upcase table maps them).
fn upcased(name: &[u8]) -> Vec<u16> {
    vfs_to_wide(name)
        .into_iter()
        .map(|u| {
            let Some(c) = char::from_u32(u32::from(u)) else { return u };
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(one), None) if (one as u32) <= 0xFFFF => one as u32 as u16,
                _ => u,
            }
        })
        .collect()
}

/// The order NTFS lists a directory in: the names in capitals, compared as UTF-16 units.
pub fn ntfs_order(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    upcased(a).cmp(&upcased(b)).then_with(|| a.cmp(b))
}

impl IoState {
    /// Run one `NtQueryDirectoryFile` call on directory file object `cookie`.
    pub fn query_directory(&mut self, cookie: u64, q: &Query<'_>) -> Result<Answer, NtStatus> {
        let fixed = info::dir_fixed(q.class).ok_or(STATUS_INVALID_INFO_CLASS)?;
        if q.length < fixed {
            return Err(STATUS_INFO_LENGTH_MISMATCH);
        }
        let vr = self.vfs()?;
        let rec = self.files.get(&cookie).ok_or(poly_ntobj::status::STATUS_INVALID_HANDLE)?;
        let disk = rec.disk().ok_or(poly_ntobj::status::STATUS_INVALID_PARAMETER)?;
        if rec.kind != poly_vfs::FileType::Directory {
            return Err(poly_ntobj::status::STATUS_INVALID_PARAMETER);
        }
        let dir = disk.node.clone();
        let at_root = disk.path.is_empty();
        let mut scan = match (&rec.scan, q.restart) {
            (Some(s), false) => s.clone(),
            (old, _) => {
                let pattern = match (q.pattern, old) {
                    (Some(p), _) => p.to_vec(),
                    (None, Some(o)) => o.pattern.clone(),
                    (None, None) => Vec::new(),
                };
                // A pattern without wildcards names one entry (in any case):
                // `FindFirstFileW` of a path, which a program may call for
                // every file it makes; reading the whole directory each time
                // made thousands of files quadratic (ADR 0091).
                let mut names = if names::is_literal(&pattern) {
                    super::walk::find_all(vr, &dir, &pattern)?
                } else {
                    super::walk::list(vr, &dir)?
                };
                if at_root {
                    names.retain(|n| n != b"." && n != b"..");
                }
                names.sort_by(|a, b| ntfs_order(a, b));
                Scan { pattern, names, next: 0, returned: false }
            }
        };
        let mut out: Vec<u8> = Vec::new();
        let mut last_start: Option<usize> = None;
        let mut status = poly_ntobj::status::STATUS_SUCCESS;
        while scan.next < scan.names.len() {
            let name = &scan.names[scan.next];
            let wide = vfs_to_wide(name);
            if !names::matches(&scan.pattern, &wide) {
                scan.next += 1;
                continue;
            }
            let Some(facts) = self.entry_facts(&dir, name) else {
                scan.next += 1;
                continue;
            };
            let entry = info::dir_entry(q.class, &DirEntryInfo { name: wide, facts });
            let start = out.len().next_multiple_of(8);
            if start + entry.len() > q.length {
                if last_start.is_none() {
                    // The first entry does not fit whole.
                    out.extend_from_slice(&entry[..q.length]);
                    status = STATUS_BUFFER_OVERFLOW;
                    scan.next += 1;
                    scan.returned = true;
                }
                break;
            }
            out.resize(start, 0);
            if let Some(prev) = last_start {
                let off = (start - prev) as u32;
                out[prev..prev + 4].copy_from_slice(&off.to_le_bytes());
            }
            out.extend_from_slice(&entry);
            last_start = Some(start);
            scan.next += 1;
            scan.returned = true;
            if q.single {
                break;
            }
        }
        let had = scan.returned;
        let rec = self.files.get_mut(&cookie).expect("checked");
        if out.is_empty() {
            rec.scan = Some(scan);
            return Err(if had { STATUS_NO_MORE_FILES } else { STATUS_NO_SUCH_FILE });
        }
        rec.scan = Some(scan);
        Ok(Answer { bytes: out, status })
    }

    /// The facts of entry `name` of `dir` (`None` if it went away).
    fn entry_facts(&self, dir: &NodeRef, name: &[u8]) -> Option<info::Facts> {
        let vr = self.root.as_ref()?;
        let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(dir), name, false).map_err(status_of).ok()?;
        self.facts(&node).ok()
    }
}

#[cfg(test)]
mod order_tests {
    use super::ntfs_order;
    use alloc::vec::Vec;

    #[test]
    fn names_are_listed_in_capitals_order() {
        let mut v: Vec<&[u8]> = alloc::vec![
            b"fc-copy.bin",
            b"fc-a.bin",
            b"..",
            b"B.txt",
            b"a.txt",
            b".",
            b"ads-new.bin",
            b"ads-main.bin",
            b"_x"
        ];
        v.sort_by(|a, b| ntfs_order(a, b));
        assert_eq!(
            v,
            alloc::vec![
                &b"."[..],
                b"..",
                b"a.txt",
                b"ads-main.bin",
                b"ads-new.bin",
                b"B.txt",
                b"fc-a.bin",
                b"fc-copy.bin",
                b"_x"
            ]
        );
    }
}
