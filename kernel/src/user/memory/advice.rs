//! Advice about a range of memory (madvise(2), ADR 0049): dropping pages
//! without changing the region map.
//!
//! Dropping a page is what unmapping it does to its frame (the memory's own
//! frame is freed, a shared frame or page-cache page loses this user, a
//! parked frame goes) while the region stays: the next access faults and
//! the region map says again what the page holds, zeros for anonymous
//! memory, the file's contents (or the shared object's) for a file or
//! shared mapping. Private changes to a file mapping are lost, as
//! madvise(2) says for `MADV_DONTNEED`.

use super::{Backing, Memory, PAGE_SIZE, Prot, Vma};
use alloc::vec::Vec;

/// What [`Memory::advise`] does with the range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advice {
    /// Nothing; the range must be mapped.
    Keep,
    /// Drop the pages of every region in the range.
    Discard,
    /// Drop the pages; every region in the range must be private anonymous
    /// memory ([`AdviceError::WrongKind`] otherwise, and nothing changes).
    DiscardAnonymous,
    /// Free the backing store of a shared writable mapping: refused
    /// ([`AdviceError::WrongKind`] for other regions,
    /// [`AdviceError::Unsupported`] for those).
    FreeBacking,
    /// `MADV_WIPEONFORK` (`true`): a child made by [`Memory::fork`] gets the
    /// range zero-filled; private anonymous memory only
    /// ([`AdviceError::WrongKind`]). `false`: `MADV_KEEPONFORK`.
    WipeOnFork(bool),
}

/// Why [`Memory::advise`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdviceError {
    /// Part of the range is not mapped (the mapped parts got the advice).
    NotMapped,
    /// A region of the range is not of the kind the advice needs.
    WrongKind,
    /// The advice is valid for the range but Poly cannot carry it out.
    Unsupported,
}

impl AdviceError {
    /// The Linux errno (madvise(2) ERRORS; values from
    /// `asm-generic/errno-base.h` and `asm-generic/errno.h`): `ENOMEM`
    /// (12), `EINVAL` (22), `EOPNOTSUPP` (95).
    pub const fn linux_errno(self) -> u16 {
        match self {
            AdviceError::NotMapped => 12,
            AdviceError::WrongKind => 22,
            AdviceError::Unsupported => 95,
        }
    }
}

impl Memory {
    /// Apply `advice` to `[addr, addr + len)` (page aligned, inside user
    /// space): see [`Advice`]. The mapped parts get it even when part of
    /// the range is not mapped, which is then reported
    /// ([`AdviceError::NotMapped`]).
    pub fn advise(&self, addr: u64, len: u64, advice: Advice) -> Result<(), AdviceError> {
        let end = addr.checked_add(len).ok_or(AdviceError::NotMapped)?;
        let mut inner = self.inner.lock();
        // The parts of the regions inside the range.
        let parts: Vec<Vma<super::FileRef>> = inner
            .map
            .regions_in(addr, end)
            .map(|v| {
                let mut part = v.clone();
                part.start = part.start.max(addr);
                part.end = part.end.min(end);
                part
            })
            .collect();
        let anonymous = |v: &Vma<super::FileRef>| matches!(v.backing, Backing::Anonymous);
        if let Advice::WipeOnFork(on) = advice {
            return match inner.map.set_wipe_on_fork(addr, len, on) {
                Ok(()) => Ok(()),
                Err(poly_vm::Error::Invalid) => Err(AdviceError::WrongKind),
                Err(_) => Err(AdviceError::NotMapped),
            };
        }
        match advice {
            Advice::Keep => {}
            Advice::Discard => self.release(&mut inner, &parts),
            Advice::DiscardAnonymous => {
                if !parts.iter().all(anonymous) {
                    return Err(AdviceError::WrongKind);
                }
                self.release(&mut inner, &parts);
            }
            Advice::WipeOnFork(_) => unreachable!("handled above"),
            Advice::FreeBacking => {
                let shared_writable = |v: &Vma<super::FileRef>| v.is_shared() && v.prot.contains(Prot::WRITE);
                if !parts.iter().all(shared_writable) {
                    return Err(AdviceError::WrongKind);
                }
                if !parts.is_empty() {
                    return Err(AdviceError::Unsupported);
                }
            }
        }
        let covered: u64 = parts.iter().map(|v| v.end - v.start).sum();
        debug_assert!(covered.is_multiple_of(PAGE_SIZE));
        if covered == len { Ok(()) } else { Err(AdviceError::NotMapped) }
    }
}

// --- Tests ------------------------------------------------------------------

mod tests {
    use super::super::{AnonShared, FileRef, Layout, MemoryImage, Placement};
    use super::*;
    use crate::arch::cpu::InterruptsOff;
    use crate::kernel_test;
    use crate::user::user_frames;
    use alloc::sync::Arc;

    const P: u64 = PAGE_SIZE;
    const ANON: u64 = 0x60_0000;
    const FILE: u64 = 0x70_0000;
    const SHARED: u64 = 0x80_0000;
    /// Two pages of file contents.
    static FILE_BYTES: [u8; 2 * PAGE_SIZE as usize] = [0x42; 2 * PAGE_SIZE as usize];

    /// Four pages of anonymous memory at [`ANON`], a private mapping of
    /// [`FILE_BYTES`] at [`FILE`] and four shared anonymous pages at
    /// [`SHARED`], each page written with its own byte.
    fn memory() -> Result<Memory, &'static str> {
        let m = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
        let file = FileRef::new(Arc::new(MemoryImage(&FILE_BYTES[..])));
        let shared = FileRef::new(Arc::new(AnonShared::new(4 * P)));
        let fixed = Placement::FixedNoReplace;
        m.map_anonymous(fixed(ANON), 4 * P, Prot::RW)
            .and_then(|_| m.map_file(fixed(FILE), 2 * P, Prot::RW, file, 0))
            .and_then(|_| m.map_shared_file(fixed(SHARED), 4 * P, Prot::RW, shared, 0))
            .map_err(|_| "could not lay out the test memory")?;
        for (base, pages) in [(ANON, 4), (FILE, 2), (SHARED, 4)] {
            for i in 0..pages {
                m.write(base + i * P, &[0x10 + i as u8; 8]).map_err(|_| "cannot write a test page")?;
            }
        }
        Ok(m)
    }

    fn byte(m: &Memory, addr: u64) -> Result<u8, &'static str> {
        let mut b = [0u8; 1];
        m.read(addr, &mut b).map_err(|_| "cannot read a test page")?;
        Ok(b[0])
    }

    kernel_test! {
        fn discard_gives_anonymous_pages_back_as_zeros_and_files_as_their_contents() -> TestResult {
            let m = memory()?;
            {
                let _irq = InterruptsOff::new();
                let (resident, frames) = (m.resident_pages(), user_frames());
                m.advise(ANON + P, 2 * P, Advice::Discard).map_err(|_| "MADV_DONTNEED failed")?;
                if m.resident_pages() != resident - 2 || frames - user_frames() != 2 {
                    return Err("discarding two anonymous pages did not free exactly their frames");
                }
            }
            let bytes = [byte(&m, ANON)?, byte(&m, ANON + P)?, byte(&m, ANON + 2 * P)?, byte(&m, ANON + 3 * P)?];
            if bytes != [0x10, 0, 0, 0x13] {
                return Err("discarded anonymous pages do not read as zeros, or others changed");
            }
            // A written page of a private file mapping reads the file again.
            m.advise(FILE, 2 * P, Advice::Discard).map_err(|_| "MADV_DONTNEED of a file mapping failed")?;
            if byte(&m, FILE)? != 0x42 || byte(&m, FILE + P)? != 0x42 {
                return Err("a discarded private file page does not read the file's contents");
            }
            // Shared anonymous memory keeps its contents: they are the object's.
            m.advise(SHARED, 4 * P, Advice::Discard).map_err(|_| "MADV_DONTNEED of shared memory failed")?;
            if byte(&m, SHARED + 3 * P)? != 0x13 {
                return Err("discarding shared anonymous pages lost their contents");
            }
            // Regions stay: the range is still mapped and writable.
            m.write(ANON + P, b"again").map_err(|_| "a discarded page cannot be written again")?;
            m.advise(ANON, 4 * P, Advice::Keep).map_err(|_| "a hint on a mapped range failed")
        }
    }

    kernel_test! {
        fn advice_checks_the_kind_of_memory_and_holes() -> TestResult {
            let m = memory()?;
            if m.advise(FILE, P, Advice::DiscardAnonymous) != Err(AdviceError::WrongKind)
                || byte(&m, FILE)? != 0x10
            {
                return Err("MADV_FREE of a file mapping was not refused, or changed it");
            }
            if m.advise(ANON, 2 * P, Advice::DiscardAnonymous).is_err() || byte(&m, ANON + P)? != 0 {
                return Err("MADV_FREE of private anonymous memory did not free it");
            }
            if m.advise(ANON, P, Advice::FreeBacking) != Err(AdviceError::WrongKind)
                || m.advise(SHARED, P, Advice::FreeBacking) != Err(AdviceError::Unsupported)
            {
                return Err("MADV_REMOVE was not refused as expected");
            }
            // A range running past the anonymous region: the mapped part
            // is discarded, the hole reported.
            if m.advise(ANON + 3 * P, 2 * P, Advice::Discard) != Err(AdviceError::NotMapped)
                || byte(&m, ANON + 3 * P)? != 0
            {
                return Err("a range with a hole was not reported, or its mapped part kept");
            }
            if m.advise(0x5000_0000, P, Advice::Keep) != Err(AdviceError::NotMapped) {
                return Err("advice on unmapped memory did not fail with ENOMEM");
            }
            let errnos = [AdviceError::NotMapped, AdviceError::WrongKind, AdviceError::Unsupported].map(|e| e.linux_errno());
            if errnos != [12, 22, 95] {
                return Err("an advice error has the wrong errno");
            }
            Ok(())
        }
    }
}
