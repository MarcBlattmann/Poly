//! Share access: whether a new open of a file may coexist with the opens
//! already there.
//!
//! The rules are those Learn documents for `IoCheckShareAccess` /
//! `IoSetShareAccess` / `IoRemoveShareAccess` and tabulates in "Creating and
//! Opening Files" (`CreateFile`'s sharing table). Each open is reduced to
//! three *access* bits — read (`FILE_READ_DATA` or `FILE_EXECUTE`), write
//! (`FILE_WRITE_DATA` or `FILE_APPEND_DATA`), delete (`DELETE`) — and three
//! *share* bits (`FILE_SHARE_READ`, `FILE_SHARE_WRITE`, `FILE_SHARE_DELETE`).
//! A new open is refused with `STATUS_SHARING_VIOLATION` when
//!
//! - it asks for an access that some existing open does not share, or
//! - some existing open has an access the new open does not share.
//!
//! An open that asks for none of the three accesses (only attributes, say)
//! is neither checked nor counted ("If the caller does not request read,
//! write or delete access, the share access is not checked", Learn:
//! "IoCheckShareAccess").

use super::consts::*;

/// The share-access state of one file: counts of open file objects with
/// each access and of those sharing each access.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShareAccess {
    pub open_count: u32,
    pub readers: u32,
    pub writers: u32,
    pub deleters: u32,
    pub shared_read: u32,
    pub shared_write: u32,
    pub shared_delete: u32,
}

/// What one open asks for, reduced to the bits sharing looks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShareRequest {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub share_read: bool,
    pub share_write: bool,
    pub share_delete: bool,
}

impl ShareRequest {
    /// From a granted access mask and a `ShareAccess` argument.
    pub fn new(access: u32, share: u32) -> ShareRequest {
        ShareRequest {
            read: access & (FILE_READ_DATA | FILE_EXECUTE) != 0,
            write: access & (FILE_WRITE_DATA | FILE_APPEND_DATA) != 0,
            delete: access & DELETE != 0,
            share_read: share & FILE_SHARE_READ != 0,
            share_write: share & FILE_SHARE_WRITE != 0,
            share_delete: share & FILE_SHARE_DELETE != 0,
        }
    }

    /// Whether the open takes part in sharing at all.
    pub fn counts(&self) -> bool {
        self.read || self.write || self.delete
    }
}

impl ShareAccess {
    /// `IoCheckShareAccess`: may `r` open the file now?
    pub fn check(&self, r: &ShareRequest) -> bool {
        if !r.counts() {
            return true;
        }
        let n = self.open_count;
        let wanted_ok =
            !(r.read && self.shared_read < n || r.write && self.shared_write < n || r.delete && self.shared_delete < n);
        let held_ok = !(self.readers > 0 && !r.share_read
            || self.writers > 0 && !r.share_write
            || self.deleters > 0 && !r.share_delete);
        wanted_ok && held_ok
    }

    /// `IoSetShareAccess`: count `r` in.
    pub fn add(&mut self, r: &ShareRequest) {
        if !r.counts() {
            return;
        }
        self.open_count += 1;
        self.readers += u32::from(r.read);
        self.writers += u32::from(r.write);
        self.deleters += u32::from(r.delete);
        self.shared_read += u32::from(r.share_read);
        self.shared_write += u32::from(r.share_write);
        self.shared_delete += u32::from(r.share_delete);
    }

    /// `IoRemoveShareAccess`: count `r` out (when its file object closes).
    pub fn remove(&mut self, r: &ShareRequest) {
        if !r.counts() {
            return;
        }
        self.open_count -= 1;
        self.readers -= u32::from(r.read);
        self.writers -= u32::from(r.write);
        self.deleters -= u32::from(r.delete);
        self.shared_read -= u32::from(r.share_read);
        self.shared_write -= u32::from(r.share_write);
        self.shared_delete -= u32::from(r.share_delete);
    }

    pub fn is_empty(&self) -> bool {
        self.open_count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = FILE_GENERIC_READ;
    const W: u32 = FILE_GENERIC_WRITE;
    const RW: u32 = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
    const SR: u32 = FILE_SHARE_READ;
    const SW: u32 = FILE_SHARE_WRITE;
    const SD: u32 = FILE_SHARE_DELETE;

    fn allowed(first: (u32, u32), second: (u32, u32)) -> bool {
        let mut s = ShareAccess::default();
        let a = ShareRequest::new(first.0, first.1);
        assert!(s.check(&a));
        s.add(&a);
        s.check(&ShareRequest::new(second.0, second.1))
    }

    /// The table of "Creating and Opening Files": the first call's access
    /// and share mode, the second call's, and whether the second succeeds.
    #[test]
    fn documented_sharing_table() {
        // First: GENERIC_READ, FILE_SHARE_READ.
        assert!(allowed((R, SR), (R, SR)));
        assert!(!allowed((R, SR), (W, SR)));
        assert!(!allowed((R, SR), (R, SW)));
        assert!(allowed((R, SR), (R, SR | SW)));
        assert!(!allowed((R, SR), (RW, SR | SW)));
        // First: GENERIC_READ, FILE_SHARE_WRITE.
        assert!(!allowed((R, SW), (R, SW)));
        assert!(!allowed((R, SW), (W, SW)));
        assert!(allowed((R, SW), (W, SR | SW)));
        assert!(!allowed((R, SW), (RW, SR | SW)));
        // First: GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE.
        assert!(allowed((R, SR | SW), (R, SR)));
        assert!(allowed((R, SR | SW), (W, SR)));
        assert!(allowed((R, SR | SW), (RW, SR)));
        assert!(!allowed((R, SR | SW), (R, SW)));
        // First: GENERIC_WRITE, FILE_SHARE_READ.
        assert!(!allowed((W, SR), (R, SR)));
        assert!(allowed((W, SR), (R, SW)));
        assert!(!allowed((W, SR), (W, SW)));
        // First: GENERIC_WRITE, FILE_SHARE_WRITE.
        assert!(allowed((W, SW), (W, SW)));
        assert!(!allowed((W, SW), (R, SW)));
        assert!(!allowed((W, SW), (W, SR)));
        // First: GENERIC_READ|GENERIC_WRITE, FILE_SHARE_READ|FILE_SHARE_WRITE.
        assert!(allowed((RW, SR | SW), (RW, SR | SW)));
        assert!(!allowed((RW, SR | SW), (R, SR)));
        // No sharing at all.
        assert!(!allowed((R, 0), (R, SR | SW | SD)));
        assert!(!allowed((R, SR | SW | SD), (R, 0)));
    }

    #[test]
    fn delete_access_and_attribute_only_opens() {
        assert!(!allowed((R, SR), (DELETE, SR)));
        assert!(allowed((R, SR | SD), (DELETE, SR)));
        assert!(!allowed((DELETE, 0), (R, SR | SW | SD)));
        // Only attributes: never checked, never counted.
        assert!(allowed((R, 0), (FILE_READ_ATTRIBUTES | SYNCHRONIZE, 0)));
        assert!(allowed((FILE_READ_ATTRIBUTES, 0), (RW, 0)));
    }

    #[test]
    fn remove_undoes_add() {
        let mut s = ShareAccess::default();
        let a = ShareRequest::new(RW, 0);
        s.add(&a);
        assert!(!s.check(&ShareRequest::new(R, SR | SW)));
        s.remove(&a);
        assert!(s.is_empty());
        assert_eq!(s, ShareAccess::default());
        assert!(s.check(&ShareRequest::new(R, 0)));
    }
}
