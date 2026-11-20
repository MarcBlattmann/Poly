//! The kernel side of the XNU personality's Mach core (ADR 0238): one
//! [`MachState`] for the machine behind a lock, and [`XnuKernel`] for the
//! calling thread of an XNU process, on the process's address space
//! (`user::Memory`, the `poly-vm` regions) and the scheduler's park and
//! unpark.
//!
//! A process's Mach task is its pid; its name space is made on its first
//! Mach call and dropped when the process exits ([`task_ended`]).

use crate::process::{self, ExitStatus, Process, ThreadFlags, Unparked};
use crate::sync::SpinLock;
use crate::time;
use crate::user::memory::{Backing as VmBacking, Memory, Placement, Prot};
use crate::user::{self};
use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use alloc::vec::Vec;
use poly_mach::exc::ThreadState64;
use poly_mach::{TaskId, ThreadId, ulock};
use poly_personality_xnu::exc;
use poly_personality_xnu::kernel::{
    Arch, Backing, Fault, Inherit, MachState, PageInfo, Region, ThreadStart, Wakeup, XnuKernel,
};

static MACH: SpinLock<Option<MachState>> = SpinLock::new(None);

const PAGE: u64 = 4096;

/// The process `pid` ended: its Mach name space goes, its ports die.
pub fn task_ended(pid: u32) {
    let woken = {
        let mut guard = MACH.lock();
        match guard.as_mut() {
            Some(m) => m.ipc.destroy_task(u64::from(pid)),
            None => Vec::new(),
        }
    };
    for t in woken {
        if let Some((_, flags)) = u32::try_from(t).ok().and_then(process::find_thread) {
            process::unpark(&flags);
        }
    }
}

/// The calling thread of an XNU process, for one system call.
pub struct MachThread<'a> {
    pub process: &'a Process,
    pub flags: Arc<ThreadFlags>,
    pub tid: u32,
}

impl MachThread<'_> {
    fn memory(&self) -> Option<Arc<Memory>> {
        user::current_memory()
    }

    fn thread_entry(&self) -> u64 {
        self.process.xnu.as_ref().map_or(0, |s| s.thread_entry.load(Ordering::SeqCst))
    }
}

fn prot(bits: u32) -> Prot {
    Prot::from_bits(u64::from(bits & 7)).expect("three bits")
}

impl XnuKernel for MachThread<'_> {
    fn task(&self) -> TaskId {
        u64::from(self.process.pid)
    }

    fn thread(&self) -> ThreadId {
        u64::from(self.tid)
    }

    fn arch(&self) -> Arch {
        Arch::X86_64
    }

    fn page_size(&self) -> u64 {
        PAGE
    }

    fn copy_in(&self, addr: u64, buf: &mut [u8]) -> Result<(), Fault> {
        user::copy_from_user(buf, addr).map_err(|_| Fault)
    }

    fn copy_out(&mut self, addr: u64, buf: &[u8]) -> Result<(), Fault> {
        user::copy_to_user(addr, buf).map_err(|_| Fault)
    }

    fn load_resident(&self, addr: u64, buf: &mut [u8]) -> Result<bool, Fault> {
        let memory = self.memory().ok_or(Fault)?;
        let mut page = [0u8; 1];
        memory.residency(addr & !(PAGE - 1), &mut page).map_err(|_| Fault)?;
        if page[0] & 1 == 0 {
            return Ok(false);
        }
        self.copy_in(addr, buf).map(|()| true)
    }

    fn shared_key(&self, addr: u64) -> Result<ulock::Key, Fault> {
        let memory = self.memory().ok_or(Fault)?;
        Ok(match memory.shared_key(addr) {
            Some((object, offset)) => ulock::Key::Shared { object, offset },
            None => ulock::Key::Private { task: self.task(), addr },
        })
    }

    fn now_ns(&self) -> u64 {
        time::now_ns()
    }

    fn block(&mut self, deadline_ns: Option<u64>) -> Wakeup {
        match process::park(self.process, &self.flags, deadline_ns) {
            Unparked::Woken => Wakeup::Woken,
            Unparked::TimedOut => Wakeup::TimedOut,
            Unparked::Interrupted => Wakeup::Signal,
        }
    }

    fn unblock(&mut self, thread: ThreadId) {
        if thread == u64::from(self.tid) {
            process::unpark(&self.flags);
        } else if let Some((_, flags)) = u32::try_from(thread).ok().and_then(process::find_thread) {
            process::unpark(&flags);
        }
    }

    fn with_mach(&mut self, f: &mut dyn FnMut(&mut MachState, &mut dyn XnuKernel)) {
        let mut guard = MACH.lock();
        let m = guard.get_or_insert_with(MachState::new);
        let (task, thread) = (self.task(), self.thread());
        if m.ipc.has_task(task) {
            m.ipc.create_thread(task, thread);
        } else {
            m.ipc.create_task(task, thread);
            // A child inherits its parent's task exception handlers.
            m.ipc.inherit_exception_ports(u64::from(self.process.ppid), task);
        }
        f(m, self);
    }

    fn kill_self(&mut self, signal: u32) {
        let status = self.process.group_exit(ExitStatus::Signaled { signal, core: false });
        let _ = user::request_exit(status);
    }

    fn regions(&self, start: u64, end: u64) -> Vec<Region> {
        let Some(memory) = self.memory() else { return Vec::new() };
        memory
            .regions()
            .iter()
            .filter(|v| v.start < end && v.end > start)
            .map(|v| Region {
                start: v.start,
                end: v.end,
                prot: u32::from(v.prot.bits()),
                max_prot: u32::from(v.max_prot.bits()),
                backing: match &v.backing {
                    VmBacking::Anonymous => Backing::Anonymous { shared: false },
                    // A shared mapping of no open file is shared anonymous
                    // memory (ADR 0041).
                    VmBacking::File { file, shared, .. } if file.source().file().is_none() => {
                        Backing::Anonymous { shared: *shared }
                    }
                    VmBacking::File { shared, .. } => {
                        Backing::File { shared: *shared, writable: v.max_prot.contains(Prot::WRITE) }
                    }
                },
            })
            .collect()
    }

    fn map_anon(&mut self, len: u64, fixed: Option<u64>, overwrite: bool) -> Result<u64, Fault> {
        let memory = self.memory().ok_or(Fault)?;
        let placement = match fixed {
            None => Placement::Hint(0),
            Some(a) if overwrite => Placement::Fixed(a),
            Some(a) => Placement::FixedNoReplace(a),
        };
        memory.map_anonymous(placement, len, Prot::RW).map_err(|_| Fault)
    }

    fn unmap(&mut self, start: u64, end: u64) {
        if let Some(memory) = self.memory() {
            let _ = memory.unmap(start, end - start);
        }
    }

    fn protect(&mut self, start: u64, end: u64, bits: u32) {
        if let Some(memory) = self.memory() {
            let _ = memory.protect(start, end - start, prot(bits));
        }
    }

    fn set_max_prot(&mut self, start: u64, end: u64, max: u32) {
        if let Some(memory) = self.memory() {
            let _ = memory.limit_max_prot(start, end - start, prot(max));
        }
    }

    fn set_inherit(&mut self, _start: u64, _end: u64, _inherit: Inherit) {
        // XNU processes do not fork yet; the inheritance takes effect with
        // fork.
    }

    fn zero(&mut self, start: u64, end: u64) {
        if let Some(memory) = self.memory() {
            let zeros = [0u8; PAGE as usize];
            let mut at = start;
            while at < end {
                let _ = memory.write_forced(at, &zeros);
                at += PAGE;
            }
        }
    }

    fn set_locked(&mut self, start: u64, end: u64, locked: bool) {
        if let Some(memory) = self.memory() {
            let _ = memory.lock(start, end - start, locked, locked);
        }
    }

    fn sync(&mut self, start: u64, end: u64, wait: bool) {
        let Some(memory) = self.memory() else { return };
        if !wait {
            return;
        }
        for v in memory.regions().iter().filter(|v| v.start < end && v.end > start) {
            if let VmBacking::File { file, shared: true, .. } = &v.backing
                && let Some(open) = file.source().file()
            {
                let _ = open.sync();
            }
        }
    }

    fn page_info(&self, addr: u64) -> PageInfo {
        let mut page = [0u8; 1];
        let resident =
            self.memory().is_some_and(|m| m.residency(addr & !(PAGE - 1), &mut page).is_ok() && page[0] & 1 != 0);
        PageInfo { resident, referenced: resident, modified: false }
    }

    fn thread_entry_registered(&self) -> bool {
        self.thread_entry() != 0
    }

    fn register_thread_entry(&mut self, entry: u64) -> Result<(), u32> {
        let state = self.process.xnu.as_ref().ok_or(poly_personality_xnu::errno::EINVAL)?;
        state
            .thread_entry
            .compare_exchange(0, entry.max(1), Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| poly_personality_xnu::errno::EINVAL)
    }

    /// A new thread of this process at the registered entry point (Poly's
    /// contract with its libpthread: `rdi` = thread structure, `rsi` =
    /// start function, `rdx` = argument, `rsp` = the caller-made stack).
    fn create_thread(&mut self, start: ThreadStart) -> Result<u64, u32> {
        let einval = poly_personality_xnu::errno::EINVAL;
        let process = crate::process::find(self.process.pid).ok_or(einval)?;
        let memory = self.memory().ok_or(einval)?;
        let entry = user::Entry { rip: self.thread_entry(), rsp: start.stack, args: [start.pthread, start.func, start.arg] };
        let tid = user::spawn_in(process, memory, entry, user::DEFAULT_HOOK).map_err(|_| 35u32)?; // EAGAIN
        let _ = crate::thread::detach(tid);
        Ok(start.pthread)
    }

    fn end_thread(&mut self) {
        let _ = user::request_exit(crate::process::ExitStatus::Exited(0));
    }

    fn current_state(&self) -> Option<ThreadState64> {
        Some(state_of(&user::registers()))
    }
}

/// The x86-64 registers of a trap frame as `x86_thread_state64_t` (segment
/// selectors fs and gs are 0: Poly uses the bases, not selectors).
fn state_of(r: &user::UserRegs) -> ThreadState64 {
    ThreadState64([
        r.rax, r.rbx, r.rcx, r.rdx, r.rdi, r.rsi, r.rbp, r.rsp, r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15,
        r.rip, r.rflags, r.cs, 0, 0,
    ])
}

/// `regs` with the general registers, rip, rflags and rsp of `s`.
fn apply(mut regs: user::UserRegs, s: &ThreadState64) -> user::UserRegs {
    let v = &s.0;
    (regs.rax, regs.rbx, regs.rcx, regs.rdx, regs.rdi, regs.rsi, regs.rbp, regs.rsp) =
        (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]);
    (regs.r8, regs.r9, regs.r10, regs.r11, regs.r12, regs.r13, regs.r14, regs.r15) =
        (v[8], v[9], v[10], v[11], v[12], v[13], v[14], v[15]);
    (regs.rip, regs.rflags) = (v[16], v[17]);
    regs
}

/// A CPU fault of the running thread of XNU process `process` that demand
/// paging did not resolve: raise its Mach exception (ADR 0238, addendum).
/// `true` if a handler took it and the thread goes on (with the registers
/// it was given); `false` for the default action.
pub fn fault(process: &Process, fault: &user::Fault) -> bool {
    let (Some(flags), Some(tid)) = (user::current_flags(), user::current_tid()) else { return false };
    let regs = user::registers();
    let mapped = matches!(fault.page, Some(user::FaultError::Segv(poly_vm::SegvCode::AccErr) | user::FaultError::Bus));
    let f = exc::Fault { vector: fault.vector, error: fault.error_code as u32, address: fault.address, mapped };
    let mut t = MachThread { process, flags, tid };
    match exc::deliver(&mut t, f, state_of(&regs)) {
        exc::Outcome::Resume(s) => user::set_registers(&apply(regs, &s)).is_ok(),
        exc::Outcome::Default => false,
    }
}
