//! Guest memory: a flat window of the guest's address space with 16 KiB
//! pages, per-page permissions, `MAP_JIT` regions and code tracking.
//!
//! The guest sees Apple silicon's page size, 16 KiB: mappings, protections
//! and the page flags below all work on 16 KiB pages, whatever the host's
//! page size is (4 KiB on x86-64).
//!
//! The window is `lo..lo + size` of guest addresses, backed by host memory
//! at `host + (addr - lo)`. One byte of flags per guest page says what the
//! guest may do there ([`R`], [`W`], [`X`]), whether the page belongs to a
//! `MAP_JIT` mapping ([`JIT`]) and whether translated code was made from it
//! ([`CODE`]). Translated code reads the same flag table before every
//! memory access (ADR 0239): an access the flags do not allow, or a write
//! to a page with [`CODE`], leaves the translated code and is done by the
//! interpreter, which faults precisely or records the page as written so
//! the code cache drops its translations.
//!
//! `MAP_JIT` pages follow the thread's write-protect mode, as
//! `pthread_jit_write_protect_np` sets it on macOS: in execute mode (the
//! default) they are readable and executable, in write mode readable and
//! writable.

use alloc::vec;
use alloc::vec::Vec;

use poly_a64::{Access, MemFault, Memory};

/// log2 of the guest page size.
pub const PAGE_SHIFT: u32 = 14;
/// The guest page size: 16 KiB, as on Apple silicon.
pub const PAGE_SIZE: u64 = 1 << PAGE_SHIFT;

/// Readable.
pub const R: u8 = 1;
/// Writable.
pub const W: u8 = 2;
/// Executable.
pub const X: u8 = 4;
/// Part of a `MAP_JIT` mapping: `W` and `X` follow the write-protect mode.
pub const JIT: u8 = 8;
/// Translated code was made from this page: a write must invalidate it.
pub const CODE: u8 = 16;

/// `MAP_JIT` from the macOS SDK's `sys/mman.h`.
pub const MAP_JIT: u32 = 0x0800;

/// Where the guest window's bytes live.
enum Backing {
    /// Owned by this value.
    Owned(#[allow(dead_code)] Vec<u8>),
    /// Host memory the caller reserved (for a large window).
    Raw,
}

/// The guest's memory.
pub struct GuestMem {
    lo: u64,
    size: u64,
    host: *mut u8,
    _backing: Backing,
    flags: Vec<u8>,
    /// The thread's `MAP_JIT` mode: true when JIT pages are writable.
    jit_writable: bool,
    /// Pages with [`CODE`] that were written since the last
    /// [`GuestMem::take_written_code`].
    written_code: Vec<u64>,
}

impl core::fmt::Debug for GuestMem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GuestMem").field("lo", &self.lo).field("size", &self.size).finish_non_exhaustive()
    }
}

/// Errors of [`GuestMem::map`] and friends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// The address or length is not a multiple of 16 KiB.
    Unaligned,
    /// The range is outside the guest window.
    OutOfRange,
}

impl GuestMem {
    /// A window of `size` bytes (a multiple of 16 KiB) at guest address `lo`,
    /// owned and zero-filled. Nothing is mapped yet.
    pub fn new(lo: u64, size: u64) -> GuestMem {
        assert!(lo.is_multiple_of(PAGE_SIZE) && size.is_multiple_of(PAGE_SIZE), "16 KiB aligned");
        let mut data = vec![0u8; size as usize];
        let host = data.as_mut_ptr();
        GuestMem {
            lo,
            size,
            host,
            _backing: Backing::Owned(data),
            flags: vec![0; (size >> PAGE_SHIFT) as usize],
            jit_writable: false,
            written_code: Vec::new(),
        }
    }

    /// A window over host memory the caller reserved.
    ///
    /// # Safety
    ///
    /// `host` must point to `size` bytes that are readable and writable (or
    /// commit on demand), zero-filled, used by nothing else and valid for
    /// the lifetime of the returned value.
    pub unsafe fn from_raw(lo: u64, size: u64, host: *mut u8) -> GuestMem {
        assert!(lo.is_multiple_of(PAGE_SIZE) && size.is_multiple_of(PAGE_SIZE), "16 KiB aligned");
        GuestMem {
            lo,
            size,
            host,
            _backing: Backing::Raw,
            flags: vec![0; (size >> PAGE_SHIFT) as usize],
            jit_writable: false,
            written_code: Vec::new(),
        }
    }

    /// The first guest address of the window.
    pub fn lo(&self) -> u64 {
        self.lo
    }

    /// The window's size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The host address of guest address `lo` (translated code adds the
    /// guest offset to it).
    pub fn host_base(&self) -> *mut u8 {
        self.host
    }

    /// The flag table, one byte per page from `lo` (translated code reads it).
    pub fn flag_table(&self) -> *const u8 {
        self.flags.as_ptr()
    }

    /// The flags of the page that holds `addr` (0 outside the window).
    pub fn page_flags(&self, addr: u64) -> u8 {
        match self.page_index(addr) {
            Some(i) => self.flags[i],
            None => 0,
        }
    }

    fn page_index(&self, addr: u64) -> Option<usize> {
        let off = addr.checked_sub(self.lo)?;
        (off < self.size).then_some((off >> PAGE_SHIFT) as usize)
    }

    fn pages(&self, addr: u64, len: u64) -> Result<core::ops::Range<usize>, MapError> {
        if !addr.is_multiple_of(PAGE_SIZE) || !len.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Unaligned);
        }
        let off = addr.checked_sub(self.lo).ok_or(MapError::OutOfRange)?;
        let end = off.checked_add(len).ok_or(MapError::OutOfRange)?;
        if end > self.size {
            return Err(MapError::OutOfRange);
        }
        Ok((off >> PAGE_SHIFT) as usize..(end >> PAGE_SHIFT) as usize)
    }

    /// Map `len` bytes at `addr` (both multiples of 16 KiB) with the
    /// protection `prot` (a combination of [`R`], [`W`], [`X`]); `flags`
    /// may contain [`MAP_JIT`]. The bytes are zeroed.
    pub fn map(&mut self, addr: u64, len: u64, prot: u8, flags: u32) -> Result<(), MapError> {
        let pages = self.pages(addr, len)?;
        let jit = flags & MAP_JIT != 0;
        for i in pages.clone() {
            if self.flags[i] & CODE != 0 {
                self.written_code.push(self.lo + ((i as u64) << PAGE_SHIFT));
            }
            self.flags[i] = if jit { JIT | self.jit_prot() } else { prot & (R | W | X) };
        }
        let start = (pages.start as u64) << PAGE_SHIFT;
        // SAFETY: `start..start + len` is inside the window (checked by `pages`).
        unsafe { core::ptr::write_bytes(self.host.add(start as usize), 0, len as usize) };
        Ok(())
    }

    /// Mark `len` bytes at `addr` (both multiples of 16 KiB) as mapped with
    /// `prot` without touching their bytes: for pages someone else already
    /// filled, such as a translated process's images, which the kernel maps
    /// and writes before the runtime starts (ADR 0257). A window from
    /// [`GuestMem::from_raw`] then never writes to host pages the guest does
    /// not use.
    pub fn set_mapped(&mut self, addr: u64, len: u64, prot: u8) -> Result<(), MapError> {
        let pages = self.pages(addr, len)?;
        for i in pages {
            if self.flags[i] & CODE != 0 {
                self.written_code.push(self.lo + ((i as u64) << PAGE_SHIFT));
            }
            self.flags[i] = prot & (R | W | X);
        }
        Ok(())
    }

    /// Unmap a range: further accesses fault.
    pub fn unmap(&mut self, addr: u64, len: u64) -> Result<(), MapError> {
        for i in self.pages(addr, len)? {
            if self.flags[i] & CODE != 0 {
                self.written_code.push(self.lo + ((i as u64) << PAGE_SHIFT));
            }
            self.flags[i] = 0;
        }
        Ok(())
    }

    /// Change the protection of mapped pages (`mprotect`). Taking away
    /// execute permission from a page with translations invalidates them.
    pub fn protect(&mut self, addr: u64, len: u64, prot: u8) -> Result<(), MapError> {
        let jit_prot = self.jit_prot();
        for i in self.pages(addr, len)? {
            let f = self.flags[i];
            if f & JIT != 0 {
                self.flags[i] = JIT | (f & CODE) | jit_prot;
                continue;
            }
            if f & CODE != 0 && prot & X == 0 {
                self.written_code.push(self.lo + ((i as u64) << PAGE_SHIFT));
                self.flags[i] = prot & (R | W | X);
            } else {
                self.flags[i] = (f & CODE) | (prot & (R | W | X));
            }
        }
        Ok(())
    }

    fn jit_prot(&self) -> u8 {
        if self.jit_writable { R | W } else { R | X }
    }

    /// The thread's `MAP_JIT` mode (`pthread_jit_write_protect_np`):
    /// `enabled` makes JIT pages executable and not writable, `!enabled`
    /// writable and not executable.
    pub fn set_jit_write_protect(&mut self, enabled: bool) {
        self.jit_writable = !enabled;
        let p = self.jit_prot();
        for (i, f) in self.flags.iter_mut().enumerate() {
            if *f & JIT != 0 {
                // Code made from a JIT page may not run while it is writable.
                if *f & CODE != 0 && !enabled {
                    self.written_code.push(self.lo + ((i as u64) << PAGE_SHIFT));
                    *f &= !CODE;
                }
                *f = JIT | (*f & CODE) | p;
            }
        }
    }

    /// Whether JIT pages are writable now.
    pub fn jit_writable(&self) -> bool {
        self.jit_writable
    }

    /// Mark the pages of `addr..addr + len` as holding translated code.
    pub fn mark_code(&mut self, addr: u64, len: u64) {
        let first = addr & !(PAGE_SIZE - 1);
        let mut a = first;
        while a < addr.saturating_add(len.max(1)) {
            if let Some(i) = self.page_index(a) {
                self.flags[i] |= CODE;
            }
            a += PAGE_SIZE;
        }
    }

    /// Report the pages of `addr..addr + len` as changed code (what
    /// `sys_icache_invalidate` and `IC IVAU` do): their translations are
    /// dropped by the code cache.
    pub fn invalidate(&mut self, addr: u64, len: u64) {
        let first = addr & !(PAGE_SIZE - 1);
        let mut a = first;
        while a < addr.saturating_add(len.max(1)) {
            if let Some(i) = self.page_index(a)
                && self.flags[i] & CODE != 0
            {
                self.flags[i] &= !CODE;
                self.written_code.push(a);
            }
            a += PAGE_SIZE;
        }
    }

    /// The pages whose translations must go, since the last call.
    pub fn take_written_code(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.written_code)
    }

    /// Whether code pages were written since the last [`GuestMem::take_written_code`].
    pub fn has_written_code(&self) -> bool {
        !self.written_code.is_empty()
    }

    /// Check every page of `addr..addr + len` for `need`; on success the
    /// offset of `addr` in the window.
    fn check(&self, addr: u64, len: usize, need: u8, access: Access) -> Result<usize, MemFault> {
        let mut a = addr;
        let end = addr.wrapping_add(len as u64);
        loop {
            match self.page_index(a) {
                Some(i) if self.flags[i] & need == need => {}
                _ => return Err(MemFault { addr: a, access, alignment: false }),
            }
            let next = (a | (PAGE_SIZE - 1)).wrapping_add(1);
            if next >= end || next == 0 {
                break;
            }
            a = next;
        }
        if end < addr && end != 0 {
            return Err(MemFault { addr, access, alignment: false });
        }
        Ok((addr - self.lo) as usize)
    }

    /// Copy bytes in without permission checks (a loader's view).
    pub fn poke(&mut self, addr: u64, data: &[u8]) -> Result<(), MapError> {
        let off = addr.checked_sub(self.lo).ok_or(MapError::OutOfRange)?;
        if off.checked_add(data.len() as u64).is_none_or(|e| e > self.size) {
            return Err(MapError::OutOfRange);
        }
        // SAFETY: the range is inside the window.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), self.host.add(off as usize), data.len()) };
        self.invalidate(addr, data.len() as u64);
        Ok(())
    }

    /// Copy bytes out without permission checks.
    pub fn peek(&self, addr: u64, buf: &mut [u8]) -> Result<(), MapError> {
        let off = addr.checked_sub(self.lo).ok_or(MapError::OutOfRange)?;
        if off.checked_add(buf.len() as u64).is_none_or(|e| e > self.size) {
            return Err(MapError::OutOfRange);
        }
        // SAFETY: the range is inside the window.
        unsafe { core::ptr::copy_nonoverlapping(self.host.add(off as usize), buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }
}

impl Memory for GuestMem {
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), MemFault> {
        let off = self.check(addr, buf.len(), R, Access::Read)?;
        // SAFETY: `check` proved the range is inside the window.
        unsafe { core::ptr::copy_nonoverlapping(self.host.add(off), buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), MemFault> {
        let off = self.check(addr, data.len(), W, Access::Write)?;
        // SAFETY: as in `read`.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), self.host.add(off), data.len()) };
        let end = addr + data.len() as u64 - 1;
        for a in [addr, end] {
            if let Some(i) = self.page_index(a)
                && self.flags[i] & CODE != 0
            {
                self.flags[i] &= !CODE;
                self.written_code.push(a & !(PAGE_SIZE - 1));
            }
        }
        Ok(())
    }

    fn fetch(&mut self, addr: u64) -> Result<u32, MemFault> {
        let off = self.check(addr, 4, R | X, Access::Fetch)?;
        let mut b = [0u8; 4];
        // SAFETY: as in `read`.
        unsafe { core::ptr::copy_nonoverlapping(self.host.add(off), b.as_mut_ptr(), 4) };
        Ok(u32::from_le_bytes(b))
    }
}
