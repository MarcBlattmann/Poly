//! Memory a substream shares with user space: its ring buffer and, for
//! mmap access, the status and control pages (ADR 0109).
//!
//! Each is shared anonymous memory ([`AnonShared`]), so `mmap(2)` of the
//! PCM device maps the very frames the kernel reads and writes. The pages
//! are made at once and kept (the object holds them), and the kernel
//! reaches them through the direct map.

use crate::arch;
use crate::user::memory::{PageSource, SharedFrame};
use crate::user::{AnonShared, memory::PAGE_SIZE};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr;

/// Bytes of memory shared with user space.
pub struct SharedBuf {
    object: Arc<AnonShared>,
    /// Every page's frame, in order (kept so the pointers below stay valid).
    frames: Vec<Arc<SharedFrame>>,
    /// Every page's address in the direct map.
    pages: Vec<*mut u8>,
    len: usize,
}

// SAFETY: the pointers name frames this value keeps alive (`frames`), in
// the kernel's direct map, which every CPU shares; the bytes have no
// invalid values. Concurrent access from user space is by design (see
// `write`).
unsafe impl Send for SharedBuf {}
// SAFETY: as for Send; `&self` methods only copy bytes.
unsafe impl Sync for SharedBuf {}

impl core::fmt::Debug for SharedBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedBuf").field("len", &self.len).finish_non_exhaustive()
    }
}

impl SharedBuf {
    /// `len` bytes (whole pages are made), zeroed. `None` without memory.
    pub fn new(len: usize) -> Option<SharedBuf> {
        let pages_n = len.div_ceil(PAGE_SIZE as usize).max(1);
        let object = Arc::new(AnonShared::new((pages_n as u64) * PAGE_SIZE));
        let mut frames = Vec::with_capacity(pages_n);
        let mut pages = Vec::with_capacity(pages_n);
        for i in 0..pages_n {
            let frame = object.shared_frame(i as u64 * PAGE_SIZE)?.ok()?;
            let virt = arch::phys_to_virt(frame.phys())? as *mut u8;
            frames.push(frame);
            pages.push(virt);
        }
        Some(SharedBuf { object, frames, pages, len })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The length a mapping may have: whole pages.
    pub fn map_len(&self) -> u64 {
        self.frames.len() as u64 * PAGE_SIZE
    }

    /// What `mmap(2)` maps.
    pub fn source(&self) -> Arc<dyn PageSource> {
        self.object.clone()
    }

    /// Visit the pieces of `offset..offset + len`, page by page: `each(page
    /// address, at, done)` for `at` bytes into the buffer and `done` bytes
    /// into the caller's data. False if the range is outside the buffer.
    fn pieces(&self, offset: usize, len: usize, mut each: impl FnMut(*mut u8, usize, usize)) -> bool {
        if offset.checked_add(len).is_none_or(|end| end > self.len) {
            return false;
        }
        let page = PAGE_SIZE as usize;
        let mut done = 0;
        while done < len {
            let at = offset + done;
            let n = (len - done).min(page - at % page);
            // SAFETY: `at / page` is a page of this buffer (the range was
            // checked) and `at % page + n <= page`.
            let p = unsafe { self.pages[at / page].add(at % page) };
            each(p, n, done);
            done += n;
        }
        true
    }

    /// Copy `data` to `offset`. False if it does not fit.
    pub fn write(&self, offset: usize, data: &[u8]) -> bool {
        self.pieces(offset, data.len(), |p, n, done| {
            // SAFETY: `p..p + n` lies in a live frame of this buffer
            // (`pieces`); user space may access the same bytes through a
            // mapping at the same time, which, bytes having no invalid
            // values, only mixes old and new bytes as a user-mode copy
            // would.
            unsafe { ptr::copy_nonoverlapping(data.as_ptr().add(done), p, n) }
        })
    }

    /// Fill `len` bytes at `offset` with repeats of `pattern`.
    pub fn fill(&self, offset: usize, len: usize, pattern: &[u8]) -> bool {
        if pattern.is_empty() {
            return false;
        }
        self.pieces(offset, len, |p, n, done| {
            for i in 0..n {
                // SAFETY: as for `write`.
                unsafe { p.add(i).write_volatile(pattern[(done + i) % pattern.len()]) }
            }
        })
    }

    /// Copy bytes at `offset` into `out`. False if out of range.
    pub fn read(&self, offset: usize, out: &mut [u8]) -> bool {
        let dst = out.as_mut_ptr();
        self.pieces(offset, out.len(), |p, n, done| {
            // SAFETY: as for `write`, the other way round; `dst + done` is
            // inside `out`.
            unsafe { ptr::copy_nonoverlapping(p, dst.add(done), n) }
        })
    }
}
