//! Sealed regions (mseal(2)) and userfaultfd(2) registrations (ADR 0122):
//! the region map keeps both per region (`poly_vm::Vma::sealed`,
//! `poly_vm::Vma::uffd`); this carries the calls to it and fills missing
//! pages for `UFFDIO_COPY` and `UFFDIO_ZEROPAGE`.

use super::{Backing, Memory, MemoryError, PAGE_SIZE, Prot, UserFrame, map_error, page_prot};

/// Why [`Memory::fill_missing`] did not fill a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillError {
    /// The page is present already (`EEXIST`).
    Present,
    /// The page is not in an anonymous region registered with the object
    /// (`ENOENT`, as ioctl_userfaultfd(2) documents for a destination
    /// outside a registered range).
    NotRegistered,
    /// No frame for the page (`ENOMEM`).
    OutOfMemory,
}

impl Memory {
    /// mseal(2) of `[addr, addr + len)`: see `poly_vm::VmMap::seal`.
    pub fn seal(&self, addr: u64, len: u64) -> Result<(), MemoryError> {
        Ok(self.inner.lock().map.seal(addr, len)?)
    }

    /// `UFFDIO_REGISTER`: missing pages of `[addr, addr + len)` fault to
    /// userfaultfd object `uffd`.
    pub fn set_userfault(&self, addr: u64, len: u64, uffd: u32) -> Result<(), MemoryError> {
        Ok(self.inner.lock().map.set_userfault(addr, len, uffd)?)
    }

    /// `UFFDIO_UNREGISTER` of `[addr, addr + len)` for object `uffd`.
    pub fn clear_userfault(&self, addr: u64, len: u64, uffd: u32) -> Result<(), MemoryError> {
        Ok(self.inner.lock().map.clear_userfault(addr, len, uffd)?)
    }

    /// Drop every registration with object `uffd` (it was closed).
    pub fn forget_userfaults(&self, uffd: u32) {
        self.inner.lock().map.forget_userfaults(Some(uffd));
    }

    /// Whether the page at `addr` is in a range registered with object
    /// `uffd`.
    pub fn userfault_registered(&self, addr: u64, uffd: u32) -> bool {
        self.inner.lock().map.userfault_registered(poly_vm::page_down(addr), uffd)
    }

    /// Whether the page at `addr` still faults to object `uffd`: it is
    /// registered with it and has no frame.
    pub fn awaits_userfault(&self, addr: u64, uffd: u32) -> bool {
        let page = poly_vm::page_down(addr);
        let inner = self.inner.lock();
        inner.map.userfault_registered(page, uffd)
            && !(inner.parked.contains_key(&page)
                || inner.shared.contains_key(&page)
                || inner.cached.contains_key(&page)
                || self.space.translate(page).is_some())
    }

    /// Fill the missing page `page` of a region registered with `uffd`
    /// with `data` (a page of bytes), or with zeros for `None`, without
    /// faulting to the object.
    pub fn fill_missing(&self, page: u64, data: Option<&[u8]>, uffd: u32) -> Result<(), FillError> {
        let mut inner = self.inner.lock();
        let Some(v) = inner.map.find(page).filter(|v| v.uffd == uffd && matches!(v.backing, Backing::Anonymous)) else {
            return Err(FillError::NotRegistered);
        };
        let prot = v.prot;
        if inner.parked.contains_key(&page) || inner.shared.contains_key(&page) || self.space.translate(page).is_some()
        {
            return Err(FillError::Present);
        }
        let mut frame = UserFrame::zeroed().ok_or(FillError::OutOfMemory)?;
        if let Some(data) = data {
            let n = data.len().min(PAGE_SIZE as usize);
            frame.bytes_mut()[..n].copy_from_slice(&data[..n]);
        }
        if prot == Prot::NONE {
            inner.parked.insert(page, frame);
            return Ok(());
        }
        self.space.map_owned(page, frame, page_prot(prot)).map_err(|e| match map_error(e) {
            super::FaultError::OutOfMemory => FillError::OutOfMemory,
            _ => FillError::NotRegistered,
        })
    }
}

// --- Tests ------------------------------------------------------------------

mod tests {
    use super::super::advice::AdviceError;
    use super::super::{Access, Advice, FaultError, Layout, Placement};
    use super::*;
    use crate::kernel_test;

    const P: u64 = PAGE_SIZE;
    const AT: u64 = 0x60_0000;

    kernel_test! {
        fn sealed_memory_refuses_changes_and_read_only_discards() -> TestResult {
            let m = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
            m.map_anonymous(Placement::FixedNoReplace(AT), 2 * P, Prot::READ).map_err(|_| "no mapping")?;
            m.seal(AT, P).map_err(|_| "mseal failed")?;
            if m.unmap(AT, 2 * P).is_ok() || m.protect(AT, P, Prot::RW).is_ok() {
                return Err("a sealed page was unmapped or re-protected");
            }
            if m.advise(AT, P, Advice::Discard) != Err(AdviceError::Sealed) || AdviceError::Sealed.linux_errno() != 1 {
                return Err("MADV_DONTNEED of sealed read-only memory was not EPERM");
            }
            if m.advise(AT + P, P, Advice::Discard).is_err() || m.unmap(AT + P, P).is_err() {
                return Err("the unsealed neighbour was refused");
            }
            Ok(())
        }
    }

    kernel_test! {
        fn userfault_pages_fault_to_the_object_until_filled() -> TestResult {
            let m = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
            m.map_anonymous(Placement::FixedNoReplace(AT), 2 * P, Prot::RW).map_err(|_| "no mapping")?;
            m.set_userfault(AT, 2 * P, 9).map_err(|_| "register failed")?;
            if m.fault(AT + 8, Access::Read) != Err(FaultError::UserFault { uffd: 9, page: AT }) {
                return Err("a missing registered page did not fault to the object");
            }
            if !m.awaits_userfault(AT, 9) || m.fill_missing(AT, Some(&[0x5a; 16]), 7) != Err(FillError::NotRegistered) {
                return Err("another object could fill the page");
            }
            m.fill_missing(AT, Some(&[0x5a; 16]), 9).map_err(|_| "UFFDIO_COPY failed")?;
            if m.fill_missing(AT, None, 9) != Err(FillError::Present) || m.awaits_userfault(AT, 9) {
                return Err("a filled page was filled again");
            }
            let mut b = [0u8; 2];
            m.read(AT + 15, &mut b).map_err(|_| "the filled page cannot be read")?;
            if b != [0x5a, 0] {
                return Err("the filled page does not hold the data");
            }
            m.forget_userfaults(9);
            m.write(AT + P, b"z").map_err(|_| "a page of a closed object's range did not zero-fill")
        }
    }
}
