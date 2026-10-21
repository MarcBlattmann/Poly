//! What identifies a word of shared memory across address spaces, for
//! shared futexes (futex(2): "the futex is shared ... between processes";
//! futex's spec sheet: shared futexes work between processes on
//! `MAP_SHARED` memory). A word in a shared mapping is named by the
//! object behind the mapping and its offset in it, so two processes that
//! map the object (after `fork`, or each on its own) at different
//! addresses find the same waiters; a word in private memory is named by
//! its address space and address, as private futexes are.

use super::{Memory, PageSource};
use poly_vm::Backing;

impl Memory {
    /// `(object, offset)` of the shared memory object behind `addr`, or
    /// `None` if `addr` is in private memory (or not mapped).
    pub fn shared_key(&self, addr: u64) -> Option<(u64, u64)> {
        let inner = self.inner.lock();
        let vma = inner.map.find(addr)?;
        match &vma.backing {
            Backing::File { file, offset, shared: true } => {
                Some((object_id(file.source().as_ref()), offset + (addr - vma.start)))
            }
            _ => None,
        }
    }
}

/// A number for `source` that every mapping of the same object shares:
/// the source's [`PageSource::object_id`], else where it lives in memory
/// (one `Arc` for every mapping of a shared anonymous object after
/// `fork`).
fn object_id(source: &dyn PageSource) -> u64 {
    source.object_id().unwrap_or(source as *const dyn PageSource as *const () as u64)
}
