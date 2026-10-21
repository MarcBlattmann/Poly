//! Host tests of the services through [`crate::dispatch_with`], against a
//! fake kernel: user memory is a map of 4 KiB pages with rights, section
//! pages are shared buffers, and a scratch area above the NT address space
//! holds the arguments a test passes by reference.

extern crate std;

mod exec;
mod memory;
mod objects;
mod sections;

use crate::backend::{Backend, SectionSource, ViewMap, ViewSource};
use crate::context::NtContext;
use crate::kernel::{Apc, ClientId, NtKernel, NtTimeout, ThreadCreate, ThreadCreated, UserFault};
use crate::pe_writer::{self, PeImage, PeSection};
use crate::protect::HwProt;
use crate::services::Service;
use crate::system::{NtProcess, NtSystem};
use crate::vm::{ADDRESS_END, PAGE_SIZE};
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use poly_abi::SyscallRegs;
use poly_ntobj::status::{STATUS_NO_YIELD_PERFORMED, STATUS_SUCCESS, STATUS_TIMEOUT, STATUS_USER_APC};
use poly_ntobj::{Caller, NtStatus, ProcessId, ThreadRef, WaitCancel, WaitId};
use std::rc::Rc;

pub const PID: ProcessId = ProcessId(7);
pub const TID: ThreadRef = ThreadRef(7);
/// Arguments passed by reference live here: outside the NT address space.
pub const SCRATCH: u64 = ADDRESS_END;
pub const SCRATCH_SIZE: u64 = 0x1_0000;
/// The fake stack pointer at the `syscall` (arguments 5+ above it).
pub const STACK: u64 = SCRATCH + SCRATCH_SIZE - 0x200;

type Frame = Rc<RefCell<Vec<u8>>>;

fn frame() -> Frame {
    Rc::new(RefCell::new(vec![0u8; PAGE_SIZE as usize]))
}

#[derive(Clone)]
struct Page {
    prot: HwProt,
    frame: Frame,
    /// A page of a `SEC_RESERVE` section: (backing, page index).
    section: Option<(u64, usize)>,
}

struct FakeSection {
    pages: Vec<Frame>,
    committed: Vec<bool>,
    deleted: bool,
}

/// The fake's memory and section store: the [`Backend`].
#[derive(Default)]
pub struct Mem {
    pages: BTreeMap<u64, Page>,
    sections: BTreeMap<u64, FakeSection>,
    next: u64,
    pub files: BTreeMap<u64, Vec<u8>>,
    pub deleted_files: Vec<u64>,
}

impl Mem {
    fn page(&self, addr: u64, write: bool) -> Result<&Page, UserFault> {
        let p = self.pages.get(&(addr & !(PAGE_SIZE - 1))).ok_or(UserFault)?;
        if !p.prot.read || (write && !p.prot.write) {
            return Err(UserFault);
        }
        if let Some((s, i)) = p.section
            && !self.sections[&s].committed[i]
        {
            return Err(UserFault);
        }
        Ok(p)
    }

    pub fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), UserFault> {
        for (i, b) in buf.iter_mut().enumerate() {
            let a = addr + i as u64;
            *b = self.page(a, false)?.frame.borrow()[(a % PAGE_SIZE) as usize];
        }
        Ok(())
    }

    pub fn write_bytes(&self, addr: u64, data: &[u8]) -> Result<(), UserFault> {
        for (i, b) in data.iter().enumerate() {
            let a = addr + i as u64;
            self.page(a, true)?.frame.borrow_mut()[(a % PAGE_SIZE) as usize] = *b;
        }
        Ok(())
    }

    pub fn bytes_at(&self, addr: u64, n: usize) -> Vec<u8> {
        let mut b = vec![0u8; n];
        self.read(addr, &mut b).unwrap();
        b
    }

    /// The rights the page at `addr` is mapped with, if it is.
    pub fn rights(&self, addr: u64) -> Option<HwProt> {
        self.pages.get(&(addr & !(PAGE_SIZE - 1))).map(|p| p.prot)
    }

    /// Section backings not yet deleted.
    pub fn live_sections(&self) -> usize {
        self.sections.values().filter(|s| !s.deleted).count()
    }

    fn pages_of(bytes: &[u8]) -> Vec<Frame> {
        bytes
            .chunks(PAGE_SIZE as usize)
            .map(|c| {
                let f = frame();
                f.borrow_mut()[..c.len()].copy_from_slice(c);
                f
            })
            .collect()
    }
}

impl Backend for Mem {
    fn commit(&mut self, addr: u64, len: u64, prot: HwProt) -> Result<(), NtStatus> {
        for a in (addr..addr + len).step_by(PAGE_SIZE as usize) {
            assert!(!self.pages.contains_key(&a), "commit over a mapped page at {a:#x}");
            self.pages.insert(a, Page { prot, frame: frame(), section: None });
        }
        Ok(())
    }

    fn release(&mut self, addr: u64, len: u64) {
        for a in (addr..addr + len).step_by(PAGE_SIZE as usize) {
            self.pages.remove(&a);
        }
    }

    fn protect(&mut self, addr: u64, len: u64, prot: HwProt) {
        for a in (addr..addr + len).step_by(PAGE_SIZE as usize) {
            if let Some(p) = self.pages.get_mut(&a) {
                p.prot = prot;
            }
        }
    }

    fn map_view(&mut self, addr: u64, len: u64, map: &ViewMap) -> Result<(), NtStatus> {
        let (frames, reserve) = match &map.source {
            ViewSource::Section(id) => {
                let s = &self.sections[id];
                assert!(!s.deleted, "mapping a deleted section");
                (s.pages.clone(), Some(*id))
            }
            ViewSource::Bytes(b) => (Mem::pages_of(b), None),
        };
        for (n, a) in (addr..addr + len).step_by(PAGE_SIZE as usize).enumerate() {
            assert!(!self.pages.contains_key(&a), "view over a mapped page at {a:#x}");
            let i = (map.offset / PAGE_SIZE) as usize + n;
            let shared = &frames[i];
            // A private view gets copies (copy on write, eagerly).
            let frame = if map.shared { Rc::clone(shared) } else { Rc::new(RefCell::new(shared.borrow().clone())) };
            let section = reserve.filter(|id| self.sections[id].committed.iter().any(|c| !c)).map(|id| (id, i));
            self.pages.insert(a, Page { prot: map.prot, frame, section });
        }
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), NtStatus> {
        self.write_bytes(addr, data).map_err(|_| poly_ntobj::status::STATUS_ACCESS_VIOLATION)
    }

    fn create_section(&mut self, source: &SectionSource) -> Result<u64, NtStatus> {
        let (pages, committed) = match source {
            SectionSource::Pagefile { size, committed } => {
                let n = size.div_ceil(PAGE_SIZE) as usize;
                ((0..n).map(|_| frame()).collect(), vec![*committed; n])
            }
            SectionSource::Image(bytes) => {
                let p = Mem::pages_of(bytes);
                let n = p.len();
                (p, vec![true; n])
            }
            SectionSource::File { file } => {
                let p = Mem::pages_of(&self.files[file]);
                let n = p.len();
                (p, vec![true; n])
            }
        };
        self.next += 1;
        self.sections.insert(self.next, FakeSection { pages, committed, deleted: false });
        Ok(self.next)
    }

    fn commit_section(&mut self, backing: u64, offset: u64, len: u64) {
        let s = self.sections.get_mut(&backing).expect("live section");
        let first = (offset / PAGE_SIZE) as usize;
        for c in &mut s.committed[first..first + (len / PAGE_SIZE) as usize] {
            *c = true;
        }
    }

    fn delete_section(&mut self, backing: u64) {
        self.sections.get_mut(&backing).expect("live section").deleted = true;
    }

    fn file_size(&mut self, file: u64) -> Option<u64> {
        self.files.get(&file).map(|f| f.len() as u64)
    }

    fn read_file(&mut self, file: u64) -> Result<Vec<u8>, NtStatus> {
        Ok(self.files[&file].clone())
    }

    fn file_deleted(&mut self, file: u64) {
        self.deleted_files.push(file);
    }
}

/// The fake kernel: one process with one thread. It does not run threads or
/// a clock, so the blocking methods resolve at once (a wait that would block
/// times out); the immediate paths and the memory/object effects are what
/// the host tests check. Threading, blocking, suspension and APC *delivery*
/// are proved by the ring-3 `kernel_test!` in the kernel crate.
pub struct Fake {
    pub sys: NtSystem,
    pub proc: NtProcess,
    pub mem: Mem,
    pub terminated: Option<NtStatus>,
    pub terminated_thread: Option<(ThreadRef, NtStatus)>,
    pub traces: Vec<[u64; 3]>,
    pub suspend: BTreeMap<ThreadRef, u32>,
    pub contexts: BTreeMap<ThreadRef, NtContext>,
    pub apcs: Vec<(ThreadRef, Apc)>,
    pub next_tid: u64,
    /// Bump allocator in the scratch area.
    next: u64,
}

impl NtKernel for Fake {
    fn read_user(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), UserFault> {
        self.mem.read(addr, buf)
    }

    fn write_user(&mut self, addr: u64, data: &[u8]) -> Result<(), UserFault> {
        self.mem.write_bytes(addr, data)
    }

    fn caller(&self) -> Caller {
        Caller::user(PID, TID)
    }

    fn with_state<R>(&mut self, f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn Backend) -> R) -> R {
        f(&mut self.sys, &mut self.proc, &mut self.mem)
    }

    fn terminate_process(&mut self, status: NtStatus) {
        self.terminated = Some(status);
    }

    fn terminate_current_thread(&mut self, status: NtStatus) {
        self.terminated_thread = Some((self.caller().thread, status));
    }

    fn create_thread(&mut self, req: ThreadCreate) -> Result<ThreadCreated, NtStatus> {
        self.next_tid += 1;
        let tid = ThreadRef(self.next_tid);
        let start = crate::exec::create_thread(&mut self.sys, &mut self.proc, &mut self.mem, &req, tid)?;
        let _ = start;
        let caller = self.caller();
        let object = self.sys.objects.thread_object(tid).expect("thread object");
        let handle = self.sys.objects.open_by_pointer(&caller, object, req.desired_access, 0)?;
        Ok(ThreadCreated { handle, client: ClientId { process: self.proc.pid, thread: tid } })
    }

    fn terminate_thread(&mut self, thread: ThreadRef, status: NtStatus) -> Result<(), NtStatus> {
        crate::exec::thread_exit(&mut self.sys, &mut self.proc, &mut self.mem, thread, status);
        Ok(())
    }

    fn suspend_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        let c = self.suspend.entry(thread).or_insert(0);
        let prev = *c;
        *c += 1;
        Ok(prev)
    }

    fn resume_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        let c = self.suspend.entry(thread).or_insert(0);
        let prev = *c;
        *c = c.saturating_sub(1);
        Ok(prev)
    }

    fn get_context(&mut self, thread: ThreadRef) -> Result<NtContext, NtStatus> {
        Ok(self.contexts.get(&thread).copied().unwrap_or_default())
    }

    fn set_context(&mut self, thread: ThreadRef, ctx: &NtContext, _want: u32) -> Result<(), NtStatus> {
        self.contexts.insert(thread, *ctx);
        Ok(())
    }

    fn wait(&mut self, id: WaitId, _timeout: NtTimeout, _alertable: bool) -> NtStatus {
        // No threads to wake it; a blocked wait times out at once here.
        self.sys.objects.cancel_wait(id, WaitCancel::Timeout).unwrap_or(STATUS_TIMEOUT)
    }

    fn arm_timer(&mut self, timer: poly_ntobj::ObjectId, _due: NtTimeout, _period: u32, _apc: Option<Apc>) -> Result<bool, NtStatus> {
        self.sys.objects.set_timer(timer)
    }

    fn disarm_timer(&mut self, timer: poly_ntobj::ObjectId) -> Result<bool, NtStatus> {
        self.sys.objects.cancel_timer(timer)
    }

    fn queue_apc(&mut self, thread: ThreadRef, apc: Apc) -> Result<(), NtStatus> {
        self.apcs.push((thread, apc));
        Ok(())
    }

    fn test_alert(&mut self) -> NtStatus {
        let me = self.caller().thread;
        if self.apcs.iter().any(|(t, _)| *t == me) { STATUS_USER_APC } else { STATUS_SUCCESS }
    }

    fn delay(&mut self, _timeout: NtTimeout, _alertable: bool) -> NtStatus {
        STATUS_SUCCESS
    }

    fn yield_execution(&mut self) -> NtStatus {
        STATUS_NO_YIELD_PERFORMED
    }

    fn resume_apc(&mut self) -> NtStatus {
        STATUS_USER_APC
    }

    fn trace(&mut self, values: [u64; 3]) {
        self.traces.push(values);
    }
}

pub const IMAGE_BASE: u64 = 0x1_4000_0000;

/// A small relocatable executable: `.text` (one `ret`), `.data` with an
/// absolute pointer to `.text` at RVA 0x2008, 0x1800 bytes in memory.
pub fn test_exe(fixed: bool) -> Vec<u8> {
    let mut data = vec![0u8; 16];
    data[8..16].copy_from_slice(&(IMAGE_BASE + 0x1000).to_le_bytes());
    PeImage {
        image_base: IMAGE_BASE,
        entry: 0x1000,
        sections: vec![
            PeSection { name: *b".text\0\0\0", data: vec![0xC3], virtual_size: 1, characteristics: pe_writer::TEXT },
            PeSection { name: *b".data\0\0\0", data, virtual_size: 0x1800, characteristics: pe_writer::DATA },
        ],
        relocations: if fixed { vec![] } else { vec![0x2008] },
        fixed,
        stack_reserve: 0x4_0000,
        stack_commit: 0x2000,
    }
    .write()
}

impl Fake {
    /// A fresh system with one empty NT process (no image).
    pub fn bare() -> Fake {
        let mut mem = Mem::default();
        for a in (SCRATCH..SCRATCH + SCRATCH_SIZE).step_by(PAGE_SIZE as usize) {
            mem.pages.insert(a, Page { prot: HwProt::RW, frame: frame(), section: None });
        }
        Fake {
            sys: NtSystem::new(),
            proc: NtProcess::new(PID),
            mem,
            terminated: None,
            terminated_thread: None,
            traces: Vec::new(),
            suspend: BTreeMap::new(),
            contexts: BTreeMap::new(),
            apcs: Vec::new(),
            next_tid: TID.0,
            next: SCRATCH,
        }
    }

    /// A process started from [`test_exe`].
    pub fn new() -> Fake {
        let mut f = Fake::bare();
        crate::exec::start_process(&mut f.sys, &mut f.proc, &mut f.mem, &test_exe(false), None, TID).expect("starts");
        f
    }

    /// Scratch space for `n` bytes, zeroed (8-aligned).
    pub fn alloc(&mut self, n: usize) -> u64 {
        let at = self.next;
        self.next += (n as u64).div_ceil(8) * 8;
        assert!(self.next <= STACK, "scratch exhausted");
        self.mem.write_bytes(at, &vec![0u8; n]).unwrap();
        at
    }

    pub fn put(&mut self, data: &[u8]) -> u64 {
        let at = self.alloc(data.len());
        self.mem.write_bytes(at, data).unwrap();
        at
    }

    pub fn put_u64(&mut self, v: u64) -> u64 {
        self.put(&v.to_le_bytes())
    }

    pub fn u64_at(&self, addr: u64) -> u64 {
        let mut b = [0u8; 8];
        self.mem.read(addr, &mut b).unwrap();
        u64::from_le_bytes(b)
    }

    pub fn u32_at(&self, addr: u64) -> u32 {
        let mut b = [0u8; 4];
        self.mem.read(addr, &mut b).unwrap();
        u32::from_le_bytes(b)
    }

    pub fn bytes_at(&self, addr: u64, n: usize) -> Vec<u8> {
        let mut b = vec![0u8; n];
        self.mem.read(addr, &mut b).unwrap();
        b
    }

    /// A `UNICODE_STRING` for `s`.
    pub fn unicode(&mut self, s: &str) -> u64 {
        let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let buf = self.put(&units);
        let header =
            crate::abi::UnicodeString { length: units.len() as u16, maximum_length: units.len() as u16, buffer: buf };
        self.put(&header.to_bytes())
    }

    /// An `OBJECT_ATTRIBUTES` naming `s` with `attributes`.
    pub fn attrs(&mut self, s: &str, attributes: u32) -> u64 {
        self.attrs_in(0, s, attributes)
    }

    /// An `OBJECT_ATTRIBUTES` for `s` relative to `root`.
    pub fn attrs_in(&mut self, root: u64, s: &str, attributes: u32) -> u64 {
        let name = self.unicode(s);
        let mut oa = [0u8; 48];
        oa[0..4].copy_from_slice(&48u32.to_le_bytes());
        oa[8..16].copy_from_slice(&root.to_le_bytes());
        oa[16..24].copy_from_slice(&name.to_le_bytes());
        oa[24..28].copy_from_slice(&attributes.to_le_bytes());
        self.put(&oa)
    }

    /// Call `service` with `args`, the way `ntdll`'s stub would.
    pub fn call(&mut self, service: Service, args: &[u64]) -> NtStatus {
        let mut regs = SyscallRegs { rax: u64::from(service.number()), rsp: STACK, ..Default::default() };
        let regs_args = [&mut regs.r10, &mut regs.rdx, &mut regs.r8, &mut regs.r9];
        for (slot, &a) in regs_args.into_iter().zip(args) {
            *slot = a;
        }
        for (i, &a) in args.iter().enumerate().skip(4) {
            self.mem.write_bytes(STACK + 0x28 + 8 * (i as u64 - 4), &a.to_le_bytes()).unwrap();
        }
        assert_eq!(crate::dispatch_with(self, &mut regs), poly_abi::Outcome::Handled);
        NtStatus(regs.rax as u32)
    }
}

/// `NtCurrentProcess()`.
pub const SELF: u64 = u64::MAX;
