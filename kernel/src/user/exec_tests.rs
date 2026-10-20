//! In-kernel tests of exec: the static executables of `test_elf.rs` run
//! from their initial stack and report what they find there and in their
//! segments ([`Seen`]); the test hook records it (dereferencing the
//! strings and `AT_RANDOM` with the user-copy routines) and the tests
//! compare it with what the psABI and getauxval(3) say they should find.
//! The dynamically linked ones (`ET_EXEC` and `ET_DYN` with `PT_INTERP`)
//! start in the test interpreter, which checks and reports the auxiliary
//! vector and jumps to the program (ADR 0049).

use super::exec::{self, ExecError, ExecParams, INITIAL_STACK, OpenInterpreter};
use super::memory::{Kind, PAGE_SIZE};
use super::test_elf::{self, Seen, TestElf};
use super::*;
use crate::kernel_test;
use crate::sync::Mutex;
use alloc::vec;
use alloc::vec::Vec;
use poly_abi::Personality;
use poly_binfmt::{Arch, Bits, Format};
use poly_elf::{
    AT_BASE, AT_CLKTCK, AT_EGID, AT_ENTRY, AT_EUID, AT_EXECFN, AT_FLAGS, AT_GID, AT_HWCAP, AT_NULL, AT_PAGESZ, AT_PHDR,
    AT_PHENT, AT_PHNUM, AT_PLATFORM, AT_RANDOM, AT_SECURE, AT_UID, DEFAULT_DYN_BASE, Elf, LoadOptions, LoadPlan,
    StackError,
};

type TestResult<T = ()> = Result<T, &'static str>;

const PID: u32 = 4444;
const EXECFN: &[u8] = b"/bin/test-elf";
const ARGV: [&[u8]; 3] = [EXECFN, b"-x", b"two words"];
const ENVP: [&[u8]; 2] = [b"HOME=/", b"POLY=1"];

fn process() -> Arc<Process> {
    Arc::new(Process::new(PID, Personality::Linux, Format::Elf { bits: Bits::B64, arch: Arch::X86_64 }))
}

fn params() -> ExecParams<'static> {
    ExecParams::new(&ARGV, &ENVP, EXECFN)
}

/// One [`test_elf::REPORT`] call, with the bytes its pointer leads to
/// (strings, `AT_RANDOM`) read by the hook.
#[derive(Clone, Debug)]
struct Report {
    kind: u64,
    a: u64,
    b: u64,
    bytes: Option<Vec<u8>>,
}

static SEEN: Mutex<Vec<Report>> = Mutex::new(Vec::new());

fn user_string(addr: u64) -> Vec<u8> {
    let mut buf = [0u8; 256];
    match strncpy_from_user(&mut buf, addr) {
        Ok(n) => buf[..n].to_vec(),
        Err(Efault) => b"<efault>".to_vec(),
    }
}

/// The test executables' system calls: [`test_elf::REPORT`] and
/// [`test_elf::EXIT`].
fn hook(_: &Process, regs: &mut SyscallRegs) {
    match regs.rax {
        test_elf::REPORT => {
            let (kind, a, b) = (regs.rdi, regs.rsi, regs.rdx);
            let is = |s: Seen| kind == s as u64;
            let bytes = if is(Seen::Arg) || is(Seen::Env) || (is(Seen::Aux) && (a == AT_EXECFN || a == AT_PLATFORM)) {
                Some(user_string(b))
            } else if is(Seen::Aux) && a == AT_RANDOM {
                let mut random = [0u8; 16];
                copy_from_user(&mut random, b).ok().map(|()| random.to_vec())
            } else {
                None
            };
            SEEN.lock().push(Report { kind, a, b, bytes });
            regs.rax = 0;
        }
        test_elf::EXIT => exit(regs.rdi as usize),
        _ => regs.rax = u64::MAX,
    }
}

/// Wait until the threads that used `memory` have been reaped.
fn released(memory: &Arc<Memory>) -> TestResult {
    if thread::wait_until(10, || Arc::strong_count(memory) == 1) {
        Ok(())
    } else {
        Err("a user thread kept its memory map after it exited")
    }
}

/// Drop the last reference to `memory` and check that its frames went.
fn tear_down(memory: Arc<Memory>) -> TestResult {
    released(&memory)?;
    let _only = crate::smp::exclusive();
    let resident = memory.resident_pages();
    let before = user_frames();
    drop(memory);
    if before - user_frames() != resident {
        return Err("tearing down an exec'd program did not free exactly its resident frames");
    }
    Ok(())
}

/// The reports of one kind, in order.
fn of(seen: &[Report], kind: Seen) -> Vec<Report> {
    seen.iter().filter(|r| r.kind == kind as u64).cloned().collect()
}

/// `e_entry` as linked.
fn link_entry(image: &[u8]) -> u64 {
    u64::from_le_bytes(image[24..32].try_into().expect("8 bytes"))
}

/// Opens [`TestElf::Ld`] for its path, as a kernel's VFS would; anything
/// else is `ENOENT`.
fn open_ld(path: &[u8]) -> Result<Arc<dyn PageSource>, ExecError> {
    if path == test_elf::LD_PATH {
        Ok(Arc::new(MemoryImage(TestElf::Ld.image())))
    } else {
        Err(ExecError::InterpreterOpen(2))
    }
}

/// Exec `which`, run it to its end, and check everything it reported.
fn run_and_check(which: TestElf, bias: u64) -> TestResult {
    let image = which.image();
    SEEN.lock().clear();
    let loaded = exec::load_dynamic(Arc::new(MemoryImage(image)), &params(), &open_ld).map_err(|e| {
        kprintln!("user: exec of the test executable failed: {e:?}");
        "exec of the test executable failed"
    })?;
    let info = loaded.info;
    let memory = Arc::clone(&loaded.memory);
    // Before it runs, only the initial stack is resident.
    let stack_pages = info.stack.used(info.stack_top).div_ceil(PAGE_SIZE);
    if memory.resident_pages() != stack_pages {
        return Err("exec populated more than the initial stack");
    }
    let base = bias + if matches!(which, TestElf::Pie | TestElf::Dyn) { 0 } else { test_elf::TEXT_BASE };
    if info.bias != bias || info.entry != bias + link_entry(image) || info.start != base || !info.warnings.is_empty() {
        return Err("exec placed the image or its entry point wrong");
    }
    // A dynamically linked program starts in its interpreter, which lies
    // in free space of the mmap area, above the program and its break.
    let dynamic = matches!(which, TestElf::Interp | TestElf::Dyn);
    match info.interp {
        Some(ld) if dynamic => {
            let layout = memory.layout();
            if loaded.entry.rip != ld.entry || ld.entry != ld.base + link_entry(TestElf::Ld.image()) {
                return Err("a dynamically linked program does not start at its interpreter's entry point");
            }
            if ld.start != ld.base || ld.end != ld.base + PAGE_SIZE || ld.start < info.end || ld.end > layout.mmap_base
            {
                kprintln!("user: interpreter at {:#x}-{:#x}, program ends at {:#x}", ld.start, ld.end, info.end);
                return Err("the interpreter is not in free space below the stack");
            }
        }
        None if !dynamic => {
            if loaded.entry.rip != info.entry {
                return Err("a static program does not start at its entry point");
            }
        }
        _ => return Err("exec loaded an interpreter for a static program, or none for a dynamic one"),
    }
    let interp_base = info.interp.map_or(0, |ld| ld.base);
    let t = spawn_in(process(), loaded.memory, loaded.entry, hook).map_err(|_| "could not spawn the program")?;
    let code = thread::join(t).map_err(|_| "join failed")?;
    let seen = core::mem::take(&mut *SEEN.lock());
    if code != test_elf::EXIT_CODE {
        kprintln!("user: the test executable ended with {code:#x} after {} reports", seen.len());
        return Err("the test executable did not run to its end");
    }

    // The initial stack: rsp at argc, 16-byte aligned (psABI §3.4.1).
    let rsp = of(&seen, Seen::Rsp);
    if rsp.len() != 1 || rsp[0].a != info.stack.rsp || !rsp[0].a.is_multiple_of(16) {
        return Err("the program did not start with rsp at argc, 16-byte aligned");
    }
    let argc = of(&seen, Seen::Argc);
    let args = of(&seen, Seen::Arg);
    let argv_end = of(&seen, Seen::ArgvEnd);
    let ok = argc.len() == 1
        && argc[0].a == ARGV.len() as u64
        && args.len() == ARGV.len()
        && args.iter().zip(ARGV).enumerate().all(|(i, (r, want))| r.a == i as u64 && r.bytes.as_deref() == Some(want))
        && argv_end.len() == 1
        && argv_end[0].b == 0;
    if !ok {
        return Err("argc or argv arrived wrong");
    }
    let envs = of(&seen, Seen::Env);
    if envs.len() != ENVP.len() || envs.iter().zip(ENVP).any(|(r, want)| r.bytes.as_deref() != Some(want)) {
        return Err("envp arrived wrong");
    }

    // The auxiliary vector (getauxval(3)), ending with AT_NULL.
    let aux = of(&seen, Seen::Aux);
    if aux.last().map(|r| (r.a, r.b)) != Some((AT_NULL, 0)) {
        return Err("the auxiliary vector does not end with AT_NULL");
    }
    let value = |ty: u64| aux.iter().find(|r| r.a == ty).map(|r| r.b);
    let expected = [
        (AT_PHDR, base + 64),
        (AT_PHENT, 56),
        (AT_PHNUM, which.phnum()),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_BASE, interp_base),
        (AT_FLAGS, 0),
        (AT_ENTRY, info.entry),
        (AT_UID, 0),
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_SECURE, 0),
        (AT_CLKTCK, 100),
        (AT_HWCAP, crate::arch::user::hwcap()),
    ];
    for (ty, want) in expected {
        if value(ty) != Some(want) {
            kprintln!("user: auxv type {ty}: {:#x?}, expected {want:#x}", value(ty));
            return Err("an auxiliary vector entry is missing or wrong");
        }
    }
    let bytes = |ty: u64| aux.iter().find(|r| r.a == ty).and_then(|r| r.bytes.clone());
    let on_stack = |addr: Option<u64>| addr.is_some_and(|a| a > info.stack.rsp && a < info.stack_top);
    if bytes(AT_EXECFN).as_deref() != Some(EXECFN) || bytes(AT_PLATFORM).as_deref() != Some(b"x86_64") {
        return Err("AT_EXECFN or AT_PLATFORM does not name the right string");
    }
    let random = bytes(AT_RANDOM).ok_or("AT_RANDOM does not point at 16 readable bytes")?;
    if random.iter().all(|&b| b == 0) || !on_stack(value(AT_RANDOM)) || !on_stack(value(AT_EXECFN)) {
        return Err("AT_RANDOM or AT_EXECFN does not point into the information block");
    }

    // What the interpreter saw, before the program ran.
    let ld_base = of(&seen, Seen::LdBase);
    let ld_entry = of(&seen, Seen::LdEntry);
    if dynamic {
        let first = seen.first().map(|r| r.kind);
        if first != Some(Seen::LdBase as u64) || ld_base.len() != 1 || ld_entry.len() != 1 {
            return Err("the interpreter did not run before the program");
        }
        if (ld_base[0].a, ld_base[0].b) != (interp_base, interp_base) {
            return Err("AT_BASE is not the interpreter's load address");
        }
        if (ld_entry[0].a, ld_entry[0].b) != (info.entry, base + 64) {
            return Err("the interpreter did not find the program's AT_ENTRY and AT_PHDR");
        }
    } else if !ld_base.is_empty() || !ld_entry.is_empty() {
        return Err("a static program ran an interpreter");
    }

    // The segments: data, zero tails, .bss, a private copy, stack growth.
    let words = |kind: Seen| of(&seen, kind).iter().map(|r| (r.a, r.b)).collect::<Vec<_>>();
    if words(Seen::Data) != [(test_elf::DATA_WORD, 0), (test_elf::DATA_WRITE, 1)] {
        return Err("the data segment did not read its file's bytes, or could not be written");
    }
    if words(Seen::Tail) != [(0, 0), (0, 1)] {
        return Err("the rest of the data segment's last file page is not zero");
    }
    if words(Seen::Bss) != [(0, 0), (test_elf::BSS_WRITE, 1)] {
        return Err(".bss is not zero, or could not be written");
    }
    if words(Seen::Stack) != [(test_elf::STACK_WRITE, 0)] {
        return Err("the stack did not grow on demand");
    }
    let file_word = |off: usize| u64::from_le_bytes(image[off..off + 8].try_into().expect("8 bytes"));
    let data_off = image.len() - 64 - 16;
    if file_word(data_off) != test_elf::DATA_WORD {
        return Err("writing the data segment changed the file");
    }
    let stack = memory.regions().into_iter().find(|v| v.kind == Kind::Stack).ok_or("no stack region")?;
    if stack.start > info.stack.rsp - test_elf::STACK_PROBE || stack.end != info.stack_top {
        return Err("the stack region did not grow down to the program's write");
    }
    // The break starts after the highest segment, and moves.
    let data_end = base + test_elf::DATA_DELTA + data_off as u64 + 16 + test_elf::BSS;
    let brk = memory.brk_start().ok_or("no program break")?;
    if brk != info.brk || brk != poly_vm::page_up(data_end).ok_or("overflow")? || info.end != brk {
        return Err("the program break does not start after the highest segment");
    }
    if memory.brk(0) != brk || memory.brk(brk + 2 * PAGE_SIZE) != brk + 2 * PAGE_SIZE {
        return Err("the program break does not move");
    }
    memory.write(brk + PAGE_SIZE, b"heap").map_err(|_| "the heap cannot be written")?;
    tear_down(memory)
}

kernel_test! {
    fn exec_runs_a_static_executable() -> TestResult {
        run_and_check(TestElf::Exec, 0)
    }
}

kernel_test! {
    fn exec_runs_a_static_pie_at_the_default_base() -> TestResult {
        run_and_check(TestElf::Pie, DEFAULT_DYN_BASE)
    }
}

kernel_test! {
    fn exec_starts_a_dynamically_linked_pie_in_its_interpreter() -> TestResult {
        run_and_check(TestElf::Dyn, DEFAULT_DYN_BASE)
    }
}

kernel_test! {
    fn exec_starts_a_dynamically_linked_executable_in_its_interpreter() -> TestResult {
        run_and_check(TestElf::Interp, 0)
    }
}

kernel_test! {
    fn exec_refuses_interpreters_it_cannot_load() -> TestResult {
        let not_elf: &'static [u8] = b"#!/bin/sh\n";
        let missing = |_: &[u8]| -> Result<Arc<dyn PageSource>, ExecError> { Err(ExecError::InterpreterOpen(2)) };
        let garbage = |_: &[u8]| -> Result<Arc<dyn PageSource>, ExecError> { Ok(Arc::new(MemoryImage(not_elf))) };
        // An interpreter that names an interpreter itself.
        let nested =
            |_: &[u8]| -> Result<Arc<dyn PageSource>, ExecError> { Ok(Arc::new(MemoryImage(TestElf::Dyn.image()))) };
        let cases: [(OpenInterpreter<'_>, ExecError, u64); 3] = [
            (&missing, ExecError::InterpreterOpen(2), 2),
            (&garbage, ExecError::BadInterpreter(poly_elf::Error::NotElf), 80),
            (&nested, ExecError::BadInterpreter(poly_elf::Error::NestedInterpreter), 80),
        ];
        for (open, error, errno) in cases {
            for which in [TestElf::Dyn, TestElf::Interp] {
                let _only = crate::smp::exclusive();
                let before = user_frames();
                let result = exec::load_dynamic(Arc::new(MemoryImage(which.image())), &params(), open).map(|_| ());
                if result != Err(error) || error.linux_errno() != errno || user_frames() != before {
                    kprintln!("user: exec gave {result:?}, expected {error:?}");
                    return Err("exec did not refuse a bad interpreter as expected");
                }
            }
        }
        // The interpreter run as the program: a static PIE.
        let ld = exec::load(Arc::new(MemoryImage(TestElf::Ld.image())), &params()).map_err(|_| "exec of ld failed")?;
        if ld.info.interp.is_some() || ld.info.bias != DEFAULT_DYN_BASE || ld.entry.rip != ld.info.entry {
            return Err("the interpreter run as a program is not loaded like a static PIE");
        }
        tear_down(ld.memory)
    }
}

kernel_test! {
    fn exec_image_starts_the_program() -> TestResult {
        SEEN.lock().clear();
        let image: Arc<[u8]> = Arc::from(TestElf::Exec.image());
        let t = exec::exec_image(process(), image, &params(), hook).map_err(|_| "exec_image failed")?;
        if thread::join(t) != Ok(test_elf::EXIT_CODE) {
            return Err("the program started by exec_image did not run to its end");
        }
        Ok(())
    }
}

kernel_test! {
    fn exec_maps_segments_lazily_with_zero_tails() -> TestResult {
        let image = TestElf::Exec.image();
        let loaded = exec::load(Arc::new(MemoryImage(image)), &params()).map_err(|_| "exec failed")?;
        let memory = loaded.memory;
        let before = memory.resident_pages();
        let elf = Elf::parse(image).map_err(|_| "the test executable does not parse")?;
        let plan = LoadPlan::new(&elf, &LoadOptions::DEFAULT).map_err(|_| "the test executable cannot be planned")?;
        // The file holds other bytes right after the text and the data: the
        // segments' last pages must not show them.
        let text_filesz = plan.mappings()[0].file.ok_or("text is not file-backed")?.len as usize;
        if image[text_filesz..text_filesz + 8].iter().all(|&b| b == 0) || image[image.len() - 1] != 0xaa {
            return Err("the test executable lacks the bytes the zero tails must hide");
        }
        let mut pages = 0;
        let (mut want, mut got) = (vec![0u8; PAGE_SIZE as usize], vec![0u8; PAGE_SIZE as usize]);
        for m in plan.mappings() {
            for page in (m.start..m.end).step_by(PAGE_SIZE as usize) {
                if !m.fill_page(image, page, &mut want) {
                    return Err("the plan cannot fill its own page");
                }
                memory.read(page, &mut got).map_err(|_| "a segment page cannot be read")?;
                if got != want {
                    kprintln!("user: page {page:#x} of the test executable differs from its plan");
                    return Err("a segment page does not hold the file's bytes followed by zeros");
                }
                pages += 1;
            }
        }
        if memory.resident_pages() != before + pages {
            return Err("reading the segments did not bring in exactly their pages");
        }
        tear_down(memory)
    }
}

/// An image that only offers `read_at`, as a file will: exec reads it to
/// parse it.
struct NotInMemory(&'static [u8]);

impl PageSource for NotInMemory {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, memory::IoError> {
        // Short reads, to exercise the loop that reads it.
        let n = buf.len().min(100);
        MemoryImage(self.0).read_at(offset, &mut buf[..n])
    }
}

kernel_test! {
    fn exec_reads_files_that_are_not_in_memory() -> TestResult {
        SEEN.lock().clear();
        let source = Arc::new(NotInMemory(TestElf::Exec.image()));
        let t = exec::exec(process(), source, &params(), hook).map_err(|_| "exec of a read-only source failed")?;
        if thread::join(t) != Ok(test_elf::EXIT_CODE) {
            return Err("a program exec'd from a file not in memory did not run to its end");
        }
        Ok(())
    }
}

kernel_test! {
    fn exec_refuses_what_it_cannot_run() -> TestResult {
        let exec_image = TestElf::Exec.image();
        let huge = vec![b'a'; 2 << 20];
        let huge_env: [&[u8]; 1] = [&huge];
        let nul_arg: [&[u8]; 1] = [b"a\0b"];
        let cases: [(&[u8], ExecParams<'_>, ExecError, u64); 5] = [
            (TestElf::Interp.image(), params(), ExecError::Interpreter, 8),
            (&exec_image[..100], params(), ExecError::Elf(poly_elf::Error::Truncated), 8),
            (b"#!/bin/sh\n", params(), ExecError::Elf(poly_elf::Error::NotElf), 8),
            (exec_image, ExecParams::new(&nul_arg, &[], EXECFN), ExecError::Arguments(StackError::InteriorNul), 22),
            (exec_image, ExecParams::new(&ARGV, &huge_env, EXECFN), ExecError::Arguments(StackError::TooSmall), 7),
        ];
        for (image, params, error, errno) in cases {
            let source = Arc::new(MemoryImage(image));
            // A failed exec leaves no frame behind.
            let _only = crate::smp::exclusive();
            let before = user_frames();
            let result = exec::load(source, &params).map(|_| ());
            if result != Err(error) || error.linux_errno() != errno || user_frames() != before {
                kprintln!("user: exec gave {result:?}, expected {error:?}");
                return Err("exec did not refuse a bad image or bad arguments as expected");
            }
        }
        if !INITIAL_STACK.is_multiple_of(PAGE_SIZE) {
            return Err("the initial stack size is not whole pages");
        }
        Ok(())
    }
}
