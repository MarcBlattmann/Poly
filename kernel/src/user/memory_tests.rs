//! In-kernel tests of demand paging: a [`Memory`] whose regions are all
//! created empty, user threads (the programs of `programs.rs`, themselves
//! mapped from a "file") that touch them, and the kernel's user copies.
//!
//! The memory map of every test:
//!
//! | Address | Region |
//! |---|---|
//! | [`CODE`] | the test programs, a private read-execute mapping of [`programs::blob`] |
//! | [`DATA`] | [`DATA_PAGES`] pages of anonymous read-write memory |
//! | [`RO`] | 4 pages of anonymous read-only memory |
//! | [`NONE`] | 4 pages of `PROT_NONE` |
//! | [`FILE`] | 4 pages mapping a 100-byte file: one page of it, then `SIGBUS` |
//! | below `USER_END` | a 16-page stack region that grows on demand |
//!
//! Frame counts are compared with interrupts off: threads are reaped (and
//! their address spaces freed) on context switches, so only then does
//! nothing else change [`user_frames`] meanwhile.

use super::memory::{Access, Kind, Layout, MapRequest, PAGE_SIZE, Placement, Prot, SegvCode};
use super::programs::{self, Call, Program};
use super::*;
use crate::arch::cpu::InterruptsOff;
use crate::kernel_test;
use crate::sync::SpinLock;
use alloc::vec;
use poly_abi::Personality;
use poly_binfmt::{Arch, Bits, Format};

const P: u64 = PAGE_SIZE;
const CODE: u64 = 0x40_0000;
const DATA: u64 = 0x60_0000;
const DATA_PAGES: u64 = 16;
const RO: u64 = 0x70_0000;
const NONE: u64 = 0x80_0000;
const FILE: u64 = 0x90_0000;
const UNMAPPED: u64 = 0x5000_0000;
const STACK_PAGES: u64 = 16;
/// The bytes of the file mapped at [`FILE`].
const FILE_BYTES: &[u8; 100] = &[0x42; 100];
const PID: u32 = 4343;
const PF: u8 = 14;

type TestResult<T = ()> = Result<T, &'static str>;

fn process() -> Arc<Process> {
    Arc::new(Process::new(PID, Personality::Linux, Format::Elf { bits: Bits::B64, arch: Arch::X86_64 }))
}

fn stack_top() -> u64 {
    Layout::default().stack_top
}

/// A memory map laid out as in the module docs, with nothing resident.
fn new_memory() -> TestResult<Arc<Memory>> {
    let memory = Memory::new(Layout::default()).map_err(|_| "no memory for a memory map")?;
    let code = FileRef::new(Arc::new(MemoryImage(programs::blob())));
    let file = FileRef::new(Arc::new(MemoryImage(FILE_BYTES)));
    let len = programs::blob().len() as u64;
    let fixed = Placement::FixedNoReplace;
    let made = memory
        .map_file(fixed(CODE), len, Prot::RX, code, 0)
        .and_then(|_| memory.map_anonymous(fixed(DATA), DATA_PAGES * P, Prot::RW))
        .and_then(|_| memory.map_anonymous(fixed(RO), 4 * P, Prot::READ))
        .and_then(|_| memory.map_anonymous(fixed(NONE), 4 * P, Prot::NONE))
        .and_then(|_| memory.map_file(fixed(FILE), 4 * P, Prot::READ, file, 0))
        .and_then(|_| memory.setup_stack(STACK_PAGES * P, Prot::RW));
    made.map_err(|_| "could not lay out the test memory map")?;
    Ok(Arc::new(memory))
}

/// What [`Call::Callback`] runs.
static CALLBACK: SpinLock<Option<fn() -> usize>> = SpinLock::new(None);

/// [`Call::Exit`] and [`Call::Callback`]; nothing else.
fn hook(_: &Process, regs: &mut SyscallRegs) {
    if regs.rax == Call::Exit as u64 {
        exit(regs.rdi as usize);
    } else if regs.rax == Call::Callback as u64 {
        let f = CALLBACK.lock().expect("no callback set");
        exit(f());
    }
    regs.rax = u64::MAX;
}

/// Run `program` with `arg` in RDI in a new thread in `memory`; its exit code.
fn run(memory: &Arc<Memory>, program: Program, arg: u64) -> TestResult<usize> {
    let entry = Entry { rip: CODE + program.offset(), rsp: stack_top(), args: [arg, 0, 0] };
    let t = spawn_in(process(), Arc::clone(memory), entry, hook).map_err(|_| "could not spawn a user thread")?;
    thread::join(t).map_err(|_| "join failed")
}

/// Wait until the threads that used `memory` have been reaped.
fn released(memory: &Arc<Memory>) -> TestResult {
    for _ in 0..1000 {
        if Arc::strong_count(memory) == 1 {
            return Ok(());
        }
        thread::yield_now();
    }
    Err("a user thread kept its memory map after it exited")
}

/// Drop the last reference to `memory` and check that exactly its
/// resident pages' frames were freed.
fn tear_down(memory: Arc<Memory>) -> TestResult {
    released(&memory)?;
    let _irq = InterruptsOff::new();
    let resident = memory.resident_pages();
    let before = user_frames();
    drop(memory);
    if before - user_frames() != resident {
        return Err("tearing down a memory map did not free exactly its resident frames");
    }
    Ok(())
}

fn rights(memory: &Memory, addr: u64) -> Option<Prot> {
    memory.space().translate(addr).map(|m| Prot::from_bits(u64::from(m.prot.bits())).unwrap_or(Prot::NONE))
}

kernel_test! {
    fn pages_come_in_on_first_touch() -> TestResult {
        let memory = new_memory()?;
        if memory.resident_pages() != 0 || memory.layout() != Layout::default() {
            return Err("a new memory map has pages before anything touched it");
        }
        // Each access is the first to its page; the code itself comes in
        // from the file the same way.
        let cases = [
            (Program::Store, DATA + 3 * P),
            (Program::Load, DATA + 5 * P),
            (Program::Load, RO + P),
            (Program::Load, FILE + 40),
            (Program::Store, stack_top() - STACK_PAGES * P - 8),
        ];
        for (program, addr) in cases {
            if run(&memory, program, addr)? != 0 {
                kprintln!("user: first touch of {addr:#x} by {program:?} failed");
                return Err("a valid first access to a lazily mapped page did not succeed");
            }
        }
        let want = [(DATA + 3 * P, Some(Prot::RW)), (DATA + 5 * P, Some(Prot::RW)), (RO + P, Some(Prot::READ))];
        if want.iter().any(|&(addr, prot)| rights(&memory, addr) != prot) {
            return Err("a faulted-in page has the wrong rights");
        }
        if rights(&memory, DATA + 4 * P).is_some() || rights(&memory, RO).is_some() {
            return Err("a page nobody touched is resident");
        }
        // The stack grew down to the page below its region.
        let stack = memory.regions().into_iter().find(|v| v.kind == Kind::Stack).ok_or("no stack region")?;
        if stack.start != stack_top() - (STACK_PAGES + 1) * P {
            return Err("the stack did not grow down by one page on a fault below it");
        }
        // What came in from the files: the code, and the file's bytes
        // followed by zeros up to the end of its page.
        let mut code = vec![0u8; programs::blob().len()];
        memory.read(CODE, &mut code).map_err(|_| "reading the code back failed")?;
        let mut file = [0xffu8; 128];
        memory.read(FILE, &mut file).map_err(|_| "reading the file page failed")?;
        if code != programs::blob() || file[..100] != FILE_BYTES[..] || file[100..].iter().any(|&b| b != 0) {
            return Err("a file-backed page does not hold the file's bytes and a zero tail");
        }
        if memory.resident_pages() < 6 {
            return Err("faulted-in pages are not counted as resident");
        }
        tear_down(memory)
    }
}

kernel_test! {
    fn wrong_accesses_still_kill() -> TestResult {
        let memory = new_memory()?;
        let killed_pf = killed(PF);
        let cases = [
            (Program::Load, RO + 2 * P, 0, "reading read-only memory"),
            (Program::Store, RO + 2 * P, killed_pf, "writing a read-only page that is present"),
            (Program::Store, RO + 3 * P, killed_pf, "writing a read-only page that is not"),
            (Program::Store, CODE, killed_pf, "writing its code"),
            (Program::Jump, DATA, killed_pf, "executing a read-write page"),
            (Program::Load, NONE, killed_pf, "reading PROT_NONE memory"),
            (Program::Load, UNMAPPED, killed_pf, "reading outside every region"),
            (Program::Store, DATA + DATA_PAGES * P, killed_pf, "writing just past a region"),
            (Program::Load, FILE + 2 * P, killed_pf, "reading a file page beyond the end of the file"),
            (Program::Store, stack_top() - (9 << 20), killed_pf, "writing below the stack limit"),
            (Program::Load, USER_START - P, killed_pf, "reading below the lowest user address"),
        ];
        for (program, addr, expected, what) in cases {
            let code = run(&memory, program, addr)?;
            if code != expected {
                kprintln!("user: {what}: exit code {code:#x}, expected {expected:#x}");
                return Err("an access ended differently than its region allows");
            }
        }
        if rights(&memory, RO + 3 * P).is_some() || rights(&memory, UNMAPPED).is_some() {
            return Err("a refused access left a page behind");
        }
        tear_down(memory)
    }
}

/// One check of `lazy_copy_checks`, given a scratch buffer.
type Check<'a> = dyn Fn(&mut [u8; 32]) -> bool + 'a;

/// The callback of `user_copies_fault_pages_in`: 1 if every check passed,
/// else 100 + the number of the first that failed.
fn lazy_copy_checks() -> usize {
    let deep = stack_top() - 2 * STACK_PAGES * P;
    let checks: [&Check<'_>; 14] = [
        // Across two pages, neither of them present yet.
        &|_| copy_to_user(DATA + 7 * P - 4, b"lazy page").is_ok(),
        &|b| copy_from_user(&mut b[..9], DATA + 7 * P - 4).is_ok() && &b[..9] == b"lazy page",
        &|b| strncpy_from_user(b, DATA + 7 * P - 4) == Ok(9),
        &|b| copy_from_user(&mut b[..16], DATA + 9 * P).is_ok() && b[..16].iter().all(|&x| x == 0),
        &|b| copy_from_user(&mut b[..16], CODE).is_ok() && b[..16] == programs::blob()[..16],
        &|b| copy_from_user(&mut b[..8], FILE + 96).is_ok() && b[..8] == [0x42, 0x42, 0x42, 0x42, 0, 0, 0, 0],
        &|b| copy_from_user(&mut b[..8], RO + 3 * P).is_ok() && b[..8] == [0; 8],
        &|_| copy_to_user(RO + 3 * P, b"x") == Err(Efault),
        &|_| copy_to_user(RO + 2 * P + 8, b"x") == Err(Efault),
        &|_| copy_to_user(CODE, b"x") == Err(Efault),
        &|b| copy_from_user(&mut b[..8], NONE) == Err(Efault) && copy_from_user(&mut b[..8], FILE + P) == Err(Efault),
        &|_| {
            copy_to_user(UNMAPPED, b"x") == Err(Efault)
                && copy_to_user(DATA + DATA_PAGES * P - 4, b"overflow") == Err(Efault)
        },
        // The stack grows for a copy as for an access from ring 3.
        &|b| copy_to_user(deep, b"deep").is_ok() && copy_from_user(&mut b[..4], deep).is_ok() && &b[..4] == b"deep",
        &|_| current_memory().is_some_and(|m| m.space().is_active()),
    ];
    let mut buf = [0u8; 32];
    for (i, check) in checks.iter().enumerate() {
        buf.fill(0xee);
        if !check(&mut buf) {
            return 100 + i;
        }
    }
    1
}

kernel_test! {
    fn user_copies_fault_pages_in() -> TestResult {
        let memory = new_memory()?;
        *CALLBACK.lock() = Some(lazy_copy_checks);
        let code = run(&memory, Program::Callback, 0)?;
        if code != 1 {
            kprintln!("user: lazy copy check {} failed", code.wrapping_sub(100));
            return Err("a user copy into or out of a lazily mapped page failed");
        }
        let mut back = [0u8; 9];
        memory.read(DATA + 7 * P - 4, &mut back).map_err(|_| "the copied bytes are gone")?;
        if &back != b"lazy page" {
            return Err("copy_to_user into lazy pages did not land in the memory map");
        }
        // A kernel thread has no memory map: its copies still fail cleanly.
        if current_memory().is_some() || copy_to_user(DATA, b"x") != Err(Efault) {
            return Err("a kernel thread's user copy did not fail with Efault");
        }
        tear_down(memory)
    }
}

/// 8 bytes that identify page `i`.
fn marker(i: u64) -> [u8; 8] {
    (0x6d61_726b_0000_0000 | i).to_le_bytes()
}

fn read_marker(memory: &Memory, addr: u64) -> Result<[u8; 8], FaultError> {
    let mut b = [0u8; 8];
    memory.read(addr, &mut b).map(|()| b)
}

/// Run `op` with interrupts off and return how many user frames it freed.
fn frames_freed(op: impl FnOnce() -> TestResult) -> TestResult<u64> {
    let _irq = InterruptsOff::new();
    let before = user_frames();
    op()?;
    before.checked_sub(user_frames()).ok_or("an operation that frees pages took frames")
}

kernel_test! {
    fn unmap_protect_and_brk_free_frames() -> TestResult {
        let memory = new_memory()?;
        let base = memory.map_anonymous(Placement::Hint(0), 16 * P, Prot::RW).map_err(|_| "mmap failed")?;
        for i in 0..8 {
            memory.write(base + i * P, &marker(i)).map_err(|_| "writing a lazily mapped page failed")?;
        }
        if memory.resident_pages() != 8 {
            return Err("writing 8 pages did not make exactly 8 resident");
        }
        // munmap: the populated pages of the range are freed, holes are fine.
        let freed = frames_freed(|| memory.unmap(base + 6 * P, 4 * P).map_err(|_| "munmap failed"))?;
        if freed != 2 || rights(&memory, base + 6 * P).is_some() || memory.resident_pages() != 6 {
            return Err("munmap did not free exactly the populated pages of its range");
        }
        if read_marker(&memory, base + 6 * P) != Err(FaultError::Segv(SegvCode::MapErr)) {
            return Err("an unmapped page can still be read");
        }
        // PROT_NONE parks populated pages and gives them back unchanged.
        memory.protect(base + 2 * P, 2 * P, Prot::NONE).map_err(|_| "mprotect(PROT_NONE) failed")?;
        if rights(&memory, base + 2 * P).is_some() || memory.resident_pages() != 6 {
            return Err("a PROT_NONE page is still mapped, or lost its frame");
        }
        if read_marker(&memory, base + 2 * P) != Err(FaultError::Segv(SegvCode::AccErr)) {
            return Err("a PROT_NONE page can be read");
        }
        memory.protect(base + 2 * P, 2 * P, Prot::READ).map_err(|_| "mprotect(PROT_READ) failed")?;
        if rights(&memory, base + 3 * P) != Some(Prot::READ) || read_marker(&memory, base + 3 * P) != Ok(marker(3)) {
            return Err("a page made accessible again lost its contents or has the wrong rights");
        }
        if memory.write(base + 3 * P, b"x") != Err(FaultError::Segv(SegvCode::AccErr)) {
            return Err("a read-only page can be written");
        }
        // Unmapping a parked page frees it too.
        memory.protect(base + 4 * P, P, Prot::NONE).map_err(|_| "mprotect(PROT_NONE) failed")?;
        if frames_freed(|| memory.unmap(base + 4 * P, P).map_err(|_| "munmap failed"))? != 1 {
            return Err("unmapping a parked page did not free it");
        }
        // A fixed mapping replaces the pages under it.
        let fixed = || memory.map_anonymous(Placement::Fixed(base), 2 * P, Prot::RW).map(|_| ());
        if frames_freed(|| fixed().map_err(|_| "MAP_FIXED failed"))? != 2 || read_marker(&memory, base) != Ok([0; 8]) {
            return Err("MAP_FIXED did not replace the pages under it with fresh ones");
        }
        // A window of a file: bytes past its limit read as zero.
        let source: Arc<dyn PageSource> = Arc::new(MemoryImage(FILE_BYTES));
        let window = FileRef::window(Arc::clone(&source), 10);
        if window.limit() != 10 || !Arc::ptr_eq(window.source(), &source) || window == FileRef::new(source.clone()) {
            return Err("a file window does not keep its source and limit");
        }
        let at = memory.map_file(Placement::Hint(0), P, Prot::READ, window, 0).map_err(|_| "mapping a window failed")?;
        let mut head = [0xffu8; 16];
        memory.read(at, &mut head).map_err(|_| "reading a window failed")?;
        if head[..10] != FILE_BYTES[..10] || head[10..] != [0; 6] || source.is_empty() {
            return Err("a file window shows bytes past its limit");
        }
        // Shared file mappings need a page cache: refused for now.
        let shared = MapRequest::file(P, Prot::READ, FileRef::new(Arc::new(MemoryImage(FILE_BYTES))), 0, true);
        if memory.map(shared) != Err(MemoryError::Unsupported) || MemoryError::Unsupported.linux_errno() != 19 {
            return Err("a shared file mapping was accepted without a page cache");
        }
        // brk: the heap starts at the page after the break's start, grows,
        // and gives its pages back when it shrinks.
        let start = 0x1000_0123;
        memory.init_brk(start).map_err(|_| "init_brk failed")?;
        let heap = poly_vm::page_up(start).ok_or("bad break")?;
        if memory.brk(0) != start || memory.brk(start + 3 * P) != start + 3 * P {
            return Err("brk did not report or move the break");
        }
        for page in [heap, heap + P, heap + 2 * P] {
            memory.write(page, &marker(page)).map_err(|_| "writing the heap failed")?;
        }
        if memory.write(heap + 3 * P, b"x").is_ok() {
            return Err("the heap reaches past the break");
        }
        if frames_freed(|| (memory.brk(start + 8) == start + 8).then_some(()).ok_or("shrinking brk failed"))? != 3 {
            return Err("shrinking the break did not free the heap pages");
        }
        // The access an operation needs is the access the region allows.
        if memory.fault(base + 12 * P, Access::Execute) != Err(FaultError::Segv(SegvCode::AccErr)) {
            return Err("an instruction fetch from read-write memory was allowed");
        }
        tear_down(memory)
    }
}

kernel_test! {
    fn exited_threads_leave_no_frames_behind() -> TestResult {
        // A thread populates pages in ring 3 and exits; reaping it drops
        // its reference, the test drops the last one and every frame goes.
        let memory = new_memory()?;
        for i in 0..4 {
            if run(&memory, Program::Store, DATA + i * 3 * P)? != 0 {
                return Err("a store to anonymous memory failed");
            }
        }
        if memory.resident_pages() < 5 {
            return Err("the stores did not populate pages");
        }
        tear_down(memory)
    }
}
