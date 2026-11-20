//! Guest threads on host threads (ADR 0257 addendum).
//!
//! The translator's state (the engine and its code cache, the guest
//! memory and its page flags, the bridge) is shared by every guest thread
//! and only touched while [`Gil`] is held: translated code of one guest
//! thread runs at a time, which keeps code invalidation and the page flags
//! trivially consistent. A thread gives the lock up after each slice of
//! the run loop and around every system call that goes to the kernel, so
//! a call that blocks (a lock wait, a read from a pipe) never stops the
//! others. The lock is a ticket lock (first come, first served) whose
//! waiters sleep in the kernel's `__ulock_wait`.

use crate::{Kernel, Shared, exit, raw, say};
use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use poly_a64::Cpu;
use poly_a64_jit::jit::Stop;
use poly_a64_xnu::run::{self, Bridge, GuestThread, YIELD};

/// `__ulock_wait` and `__ulock_wake` (BSD 515, 516), `UL_COMPARE_AND_WAIT`
/// and `ULF_WAKE_ALL` (the ulock sheets).
const ULOCK_WAIT: u64 = 515;
const ULOCK_WAKE: u64 = 516;
const UL_COMPARE_AND_WAIT: u64 = 1;
const ULF_WAKE_ALL: u64 = 0x100;

/// Blocks of translated code (or instructions, interpreted) a thread runs
/// before it lets another have the lock.
const SLICE: u64 = 1 << 16;

/// The runtime's one lock.
pub struct Gil {
    next: AtomicU32,
    serving: AtomicU32,
}

impl Gil {
    pub const fn new() -> Gil {
        Gil { next: AtomicU32::new(0), serving: AtomicU32::new(0) }
    }

    pub fn lock(&self) {
        let me = self.next.fetch_add(1, Ordering::Relaxed);
        loop {
            let s = self.serving.load(Ordering::Acquire);
            if s == me {
                return;
            }
            let _ = raw(ULOCK_WAIT, [UL_COMPARE_AND_WAIT, self.serving.as_ptr() as u64, u64::from(s), 0, 0, 0]);
        }
    }

    pub fn unlock(&self) {
        let s = self.serving.fetch_add(1, Ordering::Release).wrapping_add(1);
        if self.next.load(Ordering::Relaxed) != s {
            let _ = raw(ULOCK_WAKE, [UL_COMPARE_AND_WAIT | ULF_WAKE_ALL, self.serving.as_ptr() as u64, 0, 0, 0, 0]);
        }
    }
}

pub static GIL: Gil = Gil::new();

/// The shared state, only touched with [`GIL`] held.
pub struct Cell(pub UnsafeCell<Option<Shared>>);

// SAFETY: every access holds GIL (see `shared`).
unsafe impl Sync for Cell {}

pub static SHARED: Cell = Cell(UnsafeCell::new(None));

/// The shared state.
///
/// # Safety
/// The caller holds [`GIL`] and keeps no other reference to it.
#[allow(clippy::mut_from_ref)]
pub unsafe fn shared() -> &'static mut Shared {
    // SAFETY: the caller's contract; `start` filled it before any thread.
    unsafe { (*SHARED.0.get()).as_mut().expect("the runtime started") }
}

/// Run the guest thread `cpu` until it ends (or the process does).
pub fn guest_loop(mut cpu: Cpu) -> ! {
    GIL.lock();
    loop {
        // SAFETY: GIL is held.
        let s = unsafe { shared() };
        match s.engine.run(&mut cpu, &mut s.mem, &mut s.bridge, SLICE) {
            Stop::Budget => {
                GIL.unlock();
                GIL.lock();
            }
            Stop::Exit(YIELD) if s.bridge.pending.is_some() => {
                let mut p = s.bridge.pending.take().expect("checked");
                GIL.unlock();
                crate::syscall(&mut p.frame, p.stack);
                GIL.lock();
                Bridge::<Kernel>::complete(&mut cpu, &p);
            }
            Stop::Exit(YIELD) if s.bridge.terminated.take().is_some() => {
                GIL.unlock();
                // bsdthread_terminate: the host thread ends; its guest stack
                // stays mapped (the runtime keeps the window's flags).
                let _ = raw(361, [0; 6]);
                loop {
                    core::hint::spin_loop();
                }
            }
            stop if run::is_fault(stop, &s.bridge) && s.bridge.deliver_stop(stop, &mut cpu, &mut s.mem) => {}
            stop => {
                if let Some(status) = s.bridge.exit {
                    if s.stats {
                        say(&run::stats_line(s.kind, &s.engine, s.bridge.calls, crate::now().wrapping_sub(s.started)));
                    }
                    exit(status);
                }
                say(&run::stop_message(&s.path, stop, &cpu, s.bridge.bad_svc));
                // The process ends with the signal a native program would
                // get (SIGSEGV, SIGBUS, SIGILL, SIGTRAP, SIGSYS).
                s.bridge.kill_self(run::stop_signal(stop, &s.mem, s.bridge.bad_svc), &mut s.mem);
                // The guest ignores it or the signal was blocked: the
                // runtime faults on purpose, so the process still ends.
                // SAFETY: ud2 raises #UD, which ends this process.
                unsafe { core::arch::asm!("ud2", options(noreturn)) }
            }
        }
    }
}

/// The host stack of a new thread.
const THREAD_STACK: usize = 256 << 10;
/// `PTHREAD_START_CUSTOM`.
const PTHREAD_START_CUSTOM: u64 = 0x0100_0000;

static REGISTERED: AtomicBool = AtomicBool::new(false);

// The kernel starts a new host thread here (Poly's contract for
// bsdthread_create: rdi = the thread structure, here the guest thread's
// record; rsi = the start function; rdx = the argument).
core::arch::global_asm!(
    ".globl poly_a64_thread_entry",
    "poly_a64_thread_entry:",
    "xor ebp, ebp",
    "and rsp, -16",
    "call {main}",
    "ud2",
    main = sym thread_main,
);

unsafe extern "C" {
    fn poly_a64_thread_entry();
}

extern "C" fn thread_main(record: *mut GuestThread) -> ! {
    // SAFETY: `start_host_thread` made the record with Box::into_raw and
    // hands it to exactly one thread.
    let t = unsafe { Box::from_raw(record) };
    guest_loop(t.cpu())
}

/// Start a host thread for the guest thread `t` (called with GIL held,
/// from the bridge).
pub fn start_host_thread(t: GuestThread) -> Result<(), u64> {
    if !REGISTERED.swap(true, Ordering::SeqCst) {
        raw(366, [poly_a64_thread_entry as *const () as u64, 0, 0, 0, 0, 0])?;
    }
    let stack = Box::leak(alloc::vec![0u8; THREAD_STACK].into_boxed_slice());
    let top = (stack.as_ptr() as u64 + THREAD_STACK as u64) & !15;
    let record = Box::into_raw(Box::new(t));
    match raw(360, [1, record as u64, top, record as u64, PTHREAD_START_CUSTOM, 0]) {
        Ok(_) => Ok(()),
        Err(e) => {
            // SAFETY: the record was not handed to a thread.
            drop(unsafe { Box::from_raw(record) });
            Err(e)
        }
    }
}
