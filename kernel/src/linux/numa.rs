//! What the personality's NUMA calls, remap_file_pages(2) and
//! adjtimex(2) need from the kernel (`sys::numa`, `sys::procmem`,
//! `sys::timex`): the regions and pages of a process's memory, mapping a
//! shared region's object again at another offset, and the system's
//! clock discipline.

use crate::sync::SpinLock;
use crate::user::memory::{Backing, PageKind};
use crate::user::{self, Memory};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::kernel::{MemRegion, PageQuery, Prot};
use poly_personality_linux::sys::timex::Ntp;

/// The memory of process `pid` (the caller's own is `own`): `ESRCH` once
/// it has ended or is ending.
fn memory_of(own: i32, pid: i32) -> Result<Arc<Memory>, Errno> {
    if pid == own { user::current_memory().ok_or(Errno::ESRCH) } else { super::procaccess::memory(pid) }
}

/// `LinuxKernel::memory_regions`.
pub fn regions(own: i32, pid: i32, start: u64, end: u64) -> Result<Vec<MemRegion>, Errno> {
    let memory = memory_of(own, pid)?;
    if start >= end {
        return Ok(Vec::new());
    }
    let out = memory
        .regions()
        .into_iter()
        .filter(|v| v.end > start && v.start < end)
        .map(|v| {
            let (shared, object) = match &v.backing {
                Backing::Anonymous => (false, 0),
                Backing::File { file, shared, .. } => {
                    (*shared, Arc::as_ptr(file.source()) as *const () as usize as u64)
                }
            };
            MemRegion { start: v.start, end: v.end, prot: Prot(u32::from(v.prot.bits())), shared, object }
        })
        .collect();
    Ok(out)
}

/// `LinuxKernel::page_query`.
pub fn page_query(own: i32, pid: i32, addr: u64) -> Result<PageQuery, Errno> {
    Ok(match memory_of(own, pid)?.page_kind(addr) {
        PageKind::Unmapped => PageQuery::Unmapped,
        PageKind::Absent => PageQuery::Absent,
        PageKind::Zero => PageQuery::Zero,
        PageKind::Present => PageQuery::Present,
    })
}

/// `LinuxKernel::remap_shared`: map the object of the shared region at
/// `start` again over `[start, start + len)` from `offset` on, with the
/// region's protection and its limit on it.
pub fn remap_shared(start: u64, len: u64, offset: u64) -> Result<(), Errno> {
    let memory = user::current_memory().ok_or(Errno::ENOMEM)?;
    let regions = memory.regions();
    let v = regions.iter().find(|v| v.contains(start)).ok_or(Errno::EINVAL)?;
    let Backing::File { file, shared: true, .. } = &v.backing else { return Err(Errno::EINVAL) };
    let req = poly_vm::MapRequest::file(len, v.prot, file.clone(), offset, true)
        .with_max_prot(v.max_prot)
        .at(poly_vm::Placement::Fixed(start));
    memory.map(req).map(|_| ()).map_err(|e| Errno::new(e.linux_errno() as u16).unwrap_or(Errno::ENOMEM))
}

/// The system's clock discipline (adjtimex(2)).
static NTP: SpinLock<Option<Ntp>> = SpinLock::new(None);

/// `LinuxKernel::ntp`.
pub fn ntp() -> Ntp {
    NTP.lock().unwrap_or_default()
}

/// `LinuxKernel::set_ntp`.
pub fn set_ntp(ntp: &Ntp) {
    *NTP.lock() = Some(*ntp);
}
