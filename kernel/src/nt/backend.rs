//! The kernel's side of the NT memory and section services: it maps pages
//! into a process's `user::Memory` (ADR 0036) and keeps the pages of every
//! section. The NT personality's model (`poly_personality_nt::vm`) decides
//! what every page is; this only carries it out.
//!
//! - A pagefile section's pages are an [`AnonShared`] object, so every
//!   view (shared, in any process) maps the same frames; a copy-on-write
//!   view gets private copies that the first write forks.
//! - An image or data-file section's pages are bytes the kernel holds
//!   (`MemoryImage`): views are always private copies.
//! - Committed private memory is demand-zero anonymous memory; a guard
//!   page is committed `PROT_NONE` and made accessible when it fires.

use crate::user::Memory;
use crate::user::memory::{AnonShared, FileRef, MemoryImage, PageSource, Placement, Prot};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_NO_MEMORY, STATUS_UNSUCCESSFUL};
use poly_personality_nt::HwProt;
use poly_personality_nt::backend::{Backend, SectionSource, ViewMap, ViewSource};

/// The kernel's store of section pages, keyed by the id the NT model uses.
#[derive(Default)]
pub struct Sections {
    /// Each section's pages. A live view keeps its own reference through
    /// the `FileRef` in the kernel's memory map, so removing a backing here
    /// never pulls frames out from under a mapped view.
    map: BTreeMap<u64, Arc<dyn PageSource>>,
    /// A file object's contents, by the kernel's key for the file (the
    /// object's cookie): minimal file objects until NT file I/O exists.
    pub files: BTreeMap<u64, Arc<[u8]>>,
    next: u64,
}

impl Sections {
    fn source(&self, id: u64) -> Arc<dyn PageSource> {
        Arc::clone(self.map.get(&id).expect("live section backing"))
    }
}

fn prot(p: HwProt) -> Prot {
    let mut bits = 0u64;
    if p.read || p.write || p.execute {
        bits |= 0x1; // x86 pages are readable whenever accessible
    }
    if p.write {
        bits |= 0x2;
    }
    if p.execute {
        bits |= 0x4;
    }
    Prot::from_bits(bits).unwrap_or(Prot::NONE)
}

/// A [`Backend`] over one process's memory and the shared section store.
pub struct MemBackend<'a> {
    pub memory: &'a Arc<Memory>,
    pub sections: &'a mut Sections,
}

impl MemBackend<'_> {
    fn map_source(
        &self,
        addr: u64,
        len: u64,
        source: Arc<dyn PageSource>,
        offset: u64,
        p: HwProt,
        shared: bool,
    ) -> Result<(), NtStatus> {
        let file = FileRef::new(source);
        let r = if shared {
            self.memory.map_shared_file(Placement::Fixed(addr), len, prot(p), file, offset)
        } else {
            self.memory.map_file(Placement::Fixed(addr), len, prot(p), file, offset)
        };
        r.map(drop).map_err(|_| STATUS_NO_MEMORY)
    }
}

impl Backend for MemBackend<'_> {
    fn commit(&mut self, addr: u64, len: u64, p: HwProt) -> Result<(), NtStatus> {
        self.memory.map_anonymous(Placement::Fixed(addr), len, prot(p)).map(drop).map_err(|_| STATUS_NO_MEMORY)
    }

    fn release(&mut self, addr: u64, len: u64) {
        let _ = self.memory.unmap(addr, len);
    }

    fn protect(&mut self, addr: u64, len: u64, p: HwProt) {
        let _ = self.memory.protect(addr, len, prot(p));
    }

    fn map_view(&mut self, addr: u64, len: u64, map: &ViewMap) -> Result<(), NtStatus> {
        match &map.source {
            ViewSource::Section(id) => {
                let source = self.sections.source(*id);
                self.map_source(addr, len, source, map.offset, map.prot, map.shared)
            }
            ViewSource::Bytes(bytes) => {
                let source: Arc<dyn PageSource> = Arc::new(MemoryImage(Arc::clone(bytes)));
                self.map_source(addr, len, source, map.offset, map.prot, false)
            }
        }
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), NtStatus> {
        self.memory.write(addr, data).map_err(|_| poly_ntobj::status::STATUS_ACCESS_VIOLATION)
    }

    fn create_section(&mut self, source: &SectionSource) -> Result<u64, NtStatus> {
        let page_source: Arc<dyn PageSource> = match source {
            SectionSource::Pagefile { size, .. } => Arc::new(AnonShared::new(*size)),
            SectionSource::Image(bytes) => Arc::new(MemoryImage(Arc::clone(bytes))),
            SectionSource::File { file } => {
                let bytes = self.sections.files.get(file).ok_or(STATUS_UNSUCCESSFUL)?;
                Arc::new(MemoryImage(Arc::clone(bytes)))
            }
        };
        self.sections.next += 1;
        let id = self.sections.next;
        self.sections.map.insert(id, page_source);
        Ok(id)
    }

    fn commit_section(&mut self, _backing: u64, _offset: u64, _len: u64) {
        // Pagefile pages come in on demand (AnonShared); nothing to do.
    }

    fn delete_section(&mut self, backing: u64) {
        self.sections.map.remove(&backing);
    }

    fn file_size(&mut self, file: u64) -> Option<u64> {
        self.sections.files.get(&file).map(|f| f.len() as u64)
    }

    fn read_file(&mut self, file: u64) -> Result<Vec<u8>, NtStatus> {
        self.sections.files.get(&file).map(|f| f.to_vec()).ok_or(STATUS_UNSUCCESSFUL)
    }
}
