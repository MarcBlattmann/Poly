//! The NT personality's state: one [`NtSystem`] for the machine (the object
//! manager and the sections) and one [`NtProcess`] per NT process (its
//! address space, process and thread objects). The kernel keeps the first
//! behind one lock and each process's behind another, always taken in
//! that order ([`crate::kernel::NtKernel::with_state`]).

use crate::backend::Backend;
use crate::image::ImageSection;
use crate::protect::Protection;
use crate::vm::VirtualMemory;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_ntobj::{ObjectId, ObjectManager, ObjectType, ProcessId, ThreadRef};

/// What a section's pages are.
#[derive(Clone, Debug)]
pub enum SectionKind {
    /// Pagefile-backed memory. `committed` has one entry per page for a
    /// `SEC_RESERVE` section (which pages were committed through some
    /// view); `None` for `SEC_COMMIT`, where all are.
    Pagefile { committed: Option<Vec<bool>> },
    /// A data file mapping of a file object (the section holds a pointer
    /// reference on it).
    File { file: ObjectId },
    /// An image (`SEC_IMAGE`), from a file object if it came from one.
    Image { image: alloc::boxed::Box<ImageSection>, file: Option<ObjectId> },
}

/// A section object's data (the object manager holds the object, whose
/// cookie is the section's key here).
#[derive(Clone, Debug)]
pub struct Section {
    /// Bytes (`MaximumSize`, or the file's or image's size).
    pub size: u64,
    /// `SectionPageProtection`.
    pub protect: Protection,
    /// `AllocationAttributes` (`SEC_*`).
    pub attributes: u32,
    /// The kernel's id for the pages ([`Backend::create_section`]).
    pub backing: u64,
    pub kind: SectionKind,
}

impl Section {
    pub fn image(&self) -> Option<&ImageSection> {
        match &self.kind {
            SectionKind::Image { image, .. } => Some(image),
            _ => None,
        }
    }

    /// A `SEC_RESERVE` section.
    pub fn is_reserve(&self) -> bool {
        matches!(self.kind, SectionKind::Pagefile { committed: Some(_) })
    }
}

/// The machine-wide NT state.
#[derive(Debug)]
pub struct NtSystem {
    pub objects: ObjectManager,
    /// File I/O: volumes, file objects, share access, locks, IRPs (ADR 0067).
    pub io: crate::io::IoState,
    sections: BTreeMap<u64, Section>,
    next_key: u64,
}

impl Default for NtSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl NtSystem {
    pub fn new() -> NtSystem {
        NtSystem { objects: ObjectManager::new(), io: crate::io::IoState::new(), sections: BTreeMap::new(), next_key: 1 }
    }

    /// Store a section's data; returns its key (the object's cookie).
    pub fn add_section(&mut self, section: Section) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        self.sections.insert(key, section);
        key
    }

    pub fn section(&self, key: u64) -> Option<&Section> {
        self.sections.get(&key)
    }

    pub fn section_mut(&mut self, key: u64) -> Option<&mut Section> {
        self.sections.get_mut(&key)
    }

    /// Forget a section whose object never came to be (or was deleted):
    /// drop what it references and its pages.
    pub fn remove_section(&mut self, key: u64, b: &mut dyn Backend) {
        let Some(s) = self.sections.remove(&key) else { return };
        b.delete_section(s.backing);
        let file = match s.kind {
            SectionKind::File { file } => Some(file),
            SectionKind::Image { file, .. } => file,
            SectionKind::Pagefile { .. } => None,
        };
        if let Some(f) = file {
            let _ = self.objects.dereference(f);
        }
    }

    /// Sections stored.
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Act on objects the object manager deleted: a section's data and
    /// pages go (and with them the reference on its file, which may delete
    /// that in turn); a file's goes to the kernel. Call after anything that
    /// can drop a reference.
    pub fn reap(&mut self, b: &mut dyn Backend) {
        while let Some(d) = self.objects.pop_deleted() {
            match d.kind {
                ObjectType::Section => self.remove_section(d.cookie, b),
                ObjectType::File => {
                    b.file_deleted(d.cookie);
                    self.io.file_closed(d.cookie);
                }
                _ => {}
            }
        }
    }
}

/// One thread of an NT process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NtThread {
    pub thread: ThreadRef,
    /// Its thread object (the process holds a pointer reference).
    pub object: ObjectId,
    /// Its TEB.
    pub teb: u64,
    /// Its stack allocation.
    pub stack: u64,
}

/// The NT state of one process.
#[derive(Debug)]
pub struct NtProcess {
    pub pid: ProcessId,
    pub vm: VirtualMemory,
    /// Its process object (a pointer reference, held until it ends).
    pub object: Option<ObjectId>,
    pub threads: Vec<NtThread>,
    /// Threads that have ended and their exit statuses, for
    /// `NtQueryInformationThread` while a handle to them is still open.
    pub exited: BTreeMap<ThreadRef, poly_ntobj::NtStatus>,
    /// Its PEB.
    pub peb: u64,
    /// Where its executable is mapped.
    pub image_base: u64,
    /// The per-process thread-start trampoline ([`crate::exec`]).
    pub trampoline: u64,
    /// The per-process user-APC trampoline ([`crate::exec`]).
    pub apc_trampoline: u64,
    /// The image's stack reserve and commit, the default for new threads.
    pub default_stack_reserve: u64,
    pub default_stack_commit: u64,
    /// Its exit status once [`crate::exec::process_exit`] has run (for the
    /// kernel to report; a running process is pending).
    pub exit_status: Option<poly_ntobj::NtStatus>,
}

impl NtProcess {
    pub fn new(pid: ProcessId) -> NtProcess {
        NtProcess {
            pid,
            vm: VirtualMemory::new(),
            object: None,
            threads: Vec::new(),
            exited: BTreeMap::new(),
            peb: 0,
            image_base: 0,
            trampoline: 0,
            apc_trampoline: 0,
            default_stack_reserve: 0x1_0000,
            default_stack_commit: 0x1000,
            exit_status: None,
        }
    }

    pub fn thread(&self, thread: ThreadRef) -> Option<&NtThread> {
        self.threads.iter().find(|t| t.thread == thread)
    }

    /// A thread's exit status, if it has ended (`NtQueryInformationThread`).
    pub fn thread_exit_status(&self, thread: ThreadRef) -> Option<poly_ntobj::NtStatus> {
        self.exited.get(&thread).copied()
    }

    /// Whether `thread` is one of this process's live threads.
    pub fn has_live_thread(&self, thread: ThreadRef) -> bool {
        self.threads.iter().any(|t| t.thread == thread)
    }
}
