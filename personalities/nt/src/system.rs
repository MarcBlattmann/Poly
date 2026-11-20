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
    /// For an image: the base its first view got in some process. Every view of the section goes
    /// there, relocated, where that is free, and is `STATUS_IMAGE_NOT_AT_BASE` (not relocated)
    /// where it is not (the sheet of `NtMapViewOfSection`: why an image is at the same address in
    /// every process of a boot).
    pub chosen_base: Option<u64>,
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

/// What [`NtSystem::thread_records`] keeps of a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadRecord {
    /// The process the thread runs (ran) in.
    pub process: ProcessId,
    /// Its exit status, once it has ended.
    pub exit: Option<poly_ntobj::NtStatus>,
    /// Made with `THREAD_CREATE_FLAGS_HIDE_FROM_DEBUGGER` (`ThreadHideFromDebugger`).
    pub hidden: bool,
}

/// The machine-wide NT state.
#[derive(Debug)]
pub struct NtSystem {
    pub objects: ObjectManager,
    /// The system's commit charge: the private memory every process has committed through
    /// `NtAllocateVirtualMemory` and not freed (ADR 0250).
    pub commit_total: u64,
    /// The most that may be committed: the machine's RAM less what the kernel keeps, set by the
    /// kernel (Poly has no pagefile, so every committed page has a frame waiting for it).
    /// `u64::MAX` until then.
    pub commit_limit: u64,
    /// File I/O: volumes, file objects, share access, locks, IRPs (ADR 0067).
    pub io: crate::io::IoState,
    /// The registry, reached through the `\REGISTRY` object (ADR 0066).
    pub registry: crate::registry::NtRegistry,

    /// The exit statuses of ended processes whose process object still
    /// exists (another process holds a handle to it), for
    /// `NtQueryInformationProcess` (`GetExitCodeProcess`, ADR 0079).
    pub process_exits: BTreeMap<ProcessId, poly_ntobj::NtStatus>,

    /// Every thread whose thread object exists: its process and, once it has ended, its exit
    /// status. A handle to a thread of another process (`CreateRemoteThread`, a child's first
    /// thread) is answered from here: `GetProcessIdOfThread` and `GetExitCodeThread` still work
    /// after that process has gone (specs/windows/kernel32.dll/CreateRemoteThread.yaml,
    /// `exit_process_routine`). Forgotten with the thread object.
    pub thread_records: BTreeMap<poly_ntobj::ThreadRef, ThreadRecord>,

    /// win32k's machine-wide state: the desktop link's surfaces (ADR 0128).
    pub win32k: crate::win32k::Win32k,

    /// The job objects and which process belongs to which (ADR 0169).
    pub jobs: crate::job::Jobs,

    /// The package SIDs of the AppContainers whose tokens were made, in the order of their
    /// first use: a container's number is its place here plus one (`TokenAppContainerNumber`,
    /// ADR 0196).
    pub lowbox_sids: Vec<poly_security::sid::Sid>,

    /// The window stations and desktops and who is on them (ADR 0192).
    pub winsta: crate::winsta::WinSta,
    /// The service control manager (ADR 0221).
    pub scm: crate::scm::ScmState,
    /// What a process keeps that another can set and a child inherits: error mode, memory
    /// priority, power throttling ([`crate::procattrs`]). A process without an entry has the
    /// defaults.
    pub proc_attrs: BTreeMap<ProcessId, crate::procattrs::ProcAttrs>,
    /// The working set limits a process was given (`SetProcessWorkingSetSize`); a process without
    /// an entry has the defaults (not inherited: a child starts with them).
    pub working_sets: BTreeMap<ProcessId, poly_kernelbase::workingset::WorkingSet>,
    /// Poly's i686 `ntdll.dll` (ADR 0186), mapped into every 32-bit process as the 64-bit one is
    /// into the others; empty when the kernel has none (a 32-bit executable is then refused).
    pub ntdll32: &'static [u8],
    /// The logon the Windows programs run in (`poly.win.token`, ADR 0222): which token a process
    /// that no process made starts with. The kernel sets it at boot ([`Logon::Standard`] unless
    /// told otherwise); a new system has the administrator without a split token (ADR 0171), as
    /// the host tests expect.
    pub logon: poly_security::token::Logon,
    /// The consent broker: elevation requests waiting for the user (ADR 0222).
    pub consent: crate::consent::Broker,

    sections: BTreeMap<u64, Section>,
    next_key: u64,
}

impl Default for NtSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl NtSystem {
    /// The number the system gives a container's SID: the same every time, a later SID a
    /// larger one (the sheet of `CreateAppContainerToken`).
    pub fn lowbox_number(&mut self, sid: &poly_security::sid::Sid) -> u32 {
        let at = match self.lowbox_sids.iter().position(|s| s == sid) {
            Some(i) => i,
            None => {
                self.lowbox_sids.push(sid.clone());
                self.lowbox_sids.len() - 1
            }
        };
        at as u32 + 1
    }

    pub fn new() -> NtSystem {
        Self::with_registry(Self::default_registry(0, &mut |_| None))
    }

    /// The registry with the default hives (ADR 0066): each loaded from the
    /// hive file `load` returns for its name, else made afresh, at `now`
    /// (a `FILETIME`). A corrupt file is replaced by the defaults
    /// (`poly_registry::defaults::install`).
    pub fn default_registry(now: u64, load: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> poly_registry::Registry {
        let mut reg = poly_registry::Registry::new();
        poly_registry::defaults::install(&mut reg, now, load).expect("the default hives fit an empty registry");
        reg
    }

    /// A namespace with `reg` mounted at `\REGISTRY`.
    pub fn with_registry(reg: poly_registry::Registry) -> NtSystem {
        let mut objects = ObjectManager::new();
        let mut registry = crate::registry::NtRegistry::new(reg);
        registry.install(&mut objects);
        NtSystem {
            objects,
            io: crate::io::IoState::new(),
            registry,
            process_exits: BTreeMap::new(),
            thread_records: BTreeMap::new(),
            win32k: crate::win32k::Win32k::default(),
            jobs: crate::job::Jobs::default(),
            lowbox_sids: Vec::new(),
            winsta: crate::winsta::WinSta::default(),
            scm: crate::scm::ScmState::default(),
            proc_attrs: BTreeMap::new(),
            working_sets: BTreeMap::new(),
            ntdll32: &[],
            logon: poly_security::token::Logon::Admin,
            commit_total: 0,
            commit_limit: u64::MAX,
            consent: crate::consent::Broker::default(),
            sections: BTreeMap::new(),
            next_key: 1,
        }
    }

    /// The desktop link's device: bytes the compositor wrote. win32k takes the input and the
    /// window records, the consent broker the user's answers (ADR 0222).
    pub fn link_receive(&mut self, bytes: &[u8]) {
        self.win32k.link_receive(&mut self.objects, bytes);
        for (request, allow) in self.win32k.take_consent_answers() {
            self.consent.desktop_answered(&mut self.objects, request, allow);
        }
    }

    /// The desktop link's device: the compositor closed it. A prompt it showed is answered No.
    pub fn link_closed(&mut self) {
        self.win32k.link_closed();
        self.consent.desktop_gone(&mut self.objects);
    }

    /// Process `pid` ended with `status`: its jobs count it out and post
    /// their messages (ADR 0169).
    pub fn job_process_exited(&mut self, pid: ProcessId, status: poly_ntobj::NtStatus) {
        self.jobs.process_exited(&mut self.io, &mut self.objects, pid, status);
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
        while let Some(c) = self.objects.pop_cleaned() {
            if c.kind == ObjectType::File {
                self.io.file_cleanup(c.cookie);
            }
        }
        while let Some(d) = self.objects.pop_deleted() {
            self.objects.security_object_deleted(&d);
            match d.kind {
                ObjectType::Section => self.remove_section(d.cookie, b),
                ObjectType::File => {
                    b.file_deleted(d.cookie);
                    self.io.file_closed(d.cookie);
                }
                ObjectType::Key => self.registry.object_deleted(d.cookie, &mut self.objects),
                ObjectType::IoCompletion => self.io.port_deleted(d.cookie),
                ObjectType::Process => {
                    self.process_exits.remove(&ProcessId(d.cookie as u32));
                    self.proc_attrs.remove(&ProcessId(d.cookie as u32));
                    self.working_sets.remove(&ProcessId(d.cookie as u32));
                    self.jobs.process_forgotten(ProcessId(d.cookie as u32));
                }
                ObjectType::Thread => {
                    self.thread_records.remove(&poly_ntobj::ThreadRef(d.cookie));
                }
                ObjectType::Job => self.jobs.deleted(d.cookie),
                ObjectType::WindowStation | ObjectType::Desktop => self.winsta.deleted(&mut self.objects, d.id),
                _ => {}
            }
            // A closed file object's or a deleted job's reference on its
            // completion port.
            for port in core::mem::take(&mut self.io.released).into_iter().chain(self.jobs.take_released()) {
                let _ = self.objects.dereference(port);
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
    /// Where it started: the entry point of the image, or the start routine `CreateThread` was given
    /// (`ThreadQuerySetWin32StartAddress`).
    pub start: u64,
    /// Its priority relative to its process's class, as `ThreadBasePriority` gives it: -2 to 2, and
    /// -16 and 16 for the idle and time-critical ends.
    pub base_priority: i32,
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
    /// Where every thread starts: `ntdll`'s `LdrInitializeThunk`
    /// ([`crate::exec`], ADR 0071).
    pub thread_start: u64,
    /// Where user APCs are delivered: `ntdll`'s `KiUserApcDispatcher`.
    pub apc_dispatcher: u64,
    /// Where `ntdll` is mapped.
    pub ntdll_base: u64,
    /// The image's stack reserve and commit, the default for new threads.
    pub default_stack_reserve: u64,
    pub default_stack_commit: u64,
    /// `PROCESS_CREATION_MITIGATION_POLICY_PROHIBIT_DYNAMIC_CODE_ALWAYS_ON`:
    /// memory cannot be made executable (ADR 0169).
    pub dynamic_code_blocked: bool,
    /// Its mitigation policies (ADR 0192): the creation attributes and `SetProcessMitigationPolicy`.
    pub mitigation: poly_kernelbase::mitigation::State,
    /// Its exit status, once known: the first `NtTerminateProcess` of it
    /// sets it, else its last thread's end ([`crate::exec::thread_exit`]);
    /// a running process is pending.
    pub exit_status: Option<poly_ntobj::NtStatus>,
    /// The user-mode exception dispatcher: `ntdll`'s
    /// `KiUserExceptionDispatcher` (ADR 0068, 0071); 0 for none.
    pub exception_dispatcher: u64,
    /// The win32k lockdown (`ProcessSystemCallDisablePolicy`'s
    /// `DisallowWin32kSystemCalls`): every table-1 call fails (ADR 0128).
    /// Set by the security milestone; off for every process until then.
    pub win32k_disabled: bool,
    /// The file its executable was started from, as `(device name, path below the volume)`
    /// (set by the kernel before the process starts; kept for the process list, `crate::proclist`).
    pub image_name: Option<(Vec<u16>, Vec<u16>)>,
    /// A 32-bit process (WOW64, ADR 0186): its executable is a PE32 image, it runs Poly's i686
    /// `ntdll`, its address space ends at 2 or 4 GiB, its PEB, TEBs and parameters have the
    /// 32-bit layout and its system calls go through [`crate::wow64`].
    pub wow64: bool,
    /// Its priority class as `ProcessPriorityClass` gives it (1 idle, 2 normal, 3 high, 4 real-time,
    /// 5 below normal, 6 above normal).
    pub priority_class: u8,
    /// The process that created it (0: the kernel started it).
    pub parent: u32,
    /// What it has added to the system's commit charge (`NtSystem::commit_total`, ADR 0250),
    /// given back when it frees memory and when it ends.
    pub commit_charged: u64,
    /// Its executable's subsystem (`IMAGE_SUBSYSTEM_*`, the optional header's `Subsystem`): 2 for a
    /// GUI program, 3 for a console one; 0 before an image is mapped. `WaitForInputIdle` refuses
    /// console programs (ADR 0245).
    pub subsystem: u16,
}

impl NtProcess {
    /// Where its PEB, TEBs and parameters keep their fields.
    pub fn layout(&self) -> poly_rtl::layout::Layout {
        if self.wow64 { poly_rtl::layout::X86 } else { poly_rtl::layout::X64 }
    }

    pub fn new(pid: ProcessId) -> NtProcess {
        NtProcess {
            pid,
            vm: VirtualMemory::new(),
            object: None,
            threads: Vec::new(),
            exited: BTreeMap::new(),
            peb: 0,
            image_base: 0,
            thread_start: 0,
            apc_dispatcher: 0,
            ntdll_base: 0,
            default_stack_reserve: 0x1_0000,
            default_stack_commit: 0x1000,
            exit_status: None,
            dynamic_code_blocked: false,
            mitigation: poly_kernelbase::mitigation::State::new(),
            exception_dispatcher: 0,
            win32k_disabled: false,
            image_name: None,
            wow64: false,
            priority_class: 2,
            parent: 0,
            commit_charged: 0,
            subsystem: 0,
        }
    }

    /// Take `state` as the process's mitigation policies and update what the kernel enforces
    /// from them: the dynamic-code and win32k policies only go on (a policy that is on stays).
    pub fn set_mitigation(&mut self, state: poly_kernelbase::mitigation::State) {
        self.dynamic_code_blocked |= state.dynamic_code_blocked();
        self.win32k_disabled |= state.win32k_disabled();
        self.mitigation = state;
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
