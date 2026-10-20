//! In-kernel tests of processes and threads (ADR 0041): small programs in
//! ring 3 that fork, clone threads, exec and wait through real system
//! calls, and the kernel's books afterwards (frames, threads, files).
//!
//! Each program exits with 0 when every check it makes passed, else with
//! the number of the first failed one (90 and up), which the test names.

use crate::fs::{self, OpenFlags};
use crate::kernel_test;
use crate::process::{self, ExitStatus, Process};
use crate::time;
use crate::user::memory::{Layout, Placement, Prot};
use crate::user::{self, Entry, Memory};
use alloc::sync::Arc;
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_abi::SyscallRegs;
use poly_binfmt::{Arch, Bits, Format};
use poly_personality_linux::task::clone::{
    CLONE_CHILD_CLEARTID, CLONE_FILES, CLONE_FS, CLONE_PARENT_SETTID, CLONE_SETTLS, CLONE_SIGHAND, CLONE_SYSVSEM,
    CLONE_THREAD, CLONE_VM,
};

type TestResult<T = ()> = Result<T, &'static str>;

/// Where the programs are mapped (read, execute).
const CODE: u64 = 0x40_0000;
/// Four read-write pages of data, zeroed.
const DATA: u64 = 0x60_0000;
const DATA_LEN: u64 = 0x4000;
/// How long a test process may take, generously: a loaded host makes the
/// emulator slow.
const DEADLINE_NS: u64 = 60 * time::NANOS_PER_SEC;

/// What pthread_create asks for (clone(2)).
const THREAD_FLAGS: u64 = CLONE_VM
    | CLONE_FS
    | CLONE_FILES
    | CLONE_SIGHAND
    | CLONE_THREAD
    | CLONE_SYSVSEM
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_CLEARTID;
/// A thread without TLS or tid pointers.
const PLAIN_THREAD: u64 = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD;

global_asm!(
    r#"
    .section .rodata.linux_task_test_programs, "a", @progbits
    .balign 16
    .global task_test_start
task_test_start:

/* fork; the child checks it sees the parent's data, writes its own copy
   and exits 42; the parent looks at the zombie (waitid WNOWAIT, kill 0),
   reaps it (wait4), checks its own copy, ESRCH and ECHILD. */
    .global task_test_fork
task_test_fork:
    mov rbx, {data}
    mov qword ptr [rbx + 8], 1
    mov eax, 57
    syscall
    test rax, rax
    jz 1f
    js 90f
    mov r12, rax
    mov edi, 1
    mov rsi, r12
    lea rdx, [rbx + 0x100]
    mov r10d, 0x01000004
    xor r8d, r8d
    mov eax, 247
    syscall
    test rax, rax
    jnz 91f
    cmp dword ptr [rbx + 0x100 + 16], r12d
    jne 92f
    cmp dword ptr [rbx + 0x100 + 8], 1
    jne 92f
    cmp dword ptr [rbx + 0x100 + 24], 42
    jne 92f
    mov rdi, r12
    xor esi, esi
    mov eax, 62
    syscall
    test rax, rax
    jnz 93f
    mov rdi, r12
    mov rsi, rbx
    xor edx, edx
    xor r10d, r10d
    mov eax, 61
    syscall
    cmp rax, r12
    jne 94f
    cmp dword ptr [rbx], 0x2a00
    jne 95f
    cmp qword ptr [rbx + 8], 1
    jne 96f
    mov rdi, r12
    xor esi, esi
    mov eax, 62
    syscall
    cmp rax, -3
    jne 97f
    mov rdi, -1
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    mov eax, 61
    syscall
    cmp rax, -10
    jne 98f
    xor edi, edi
    jmp 99f
1:
    cmp qword ptr [rbx + 8], 1
    jne 2f
    mov qword ptr [rbx + 8], 2
    cmp qword ptr [rbx + 8], 2
    jne 2f
    mov edi, 42
    jmp 99f
2:
    mov edi, 1
    jmp 99f
90: mov edi, 90
    jmp 99f
91: mov edi, 91
    jmp 99f
92: mov edi, 92
    jmp 99f
93: mov edi, 93
    jmp 99f
94: mov edi, 94
    jmp 99f
95: mov edi, 95
    jmp 99f
96: mov edi, 96
    jmp 99f
97: mov edi, 97
    jmp 99f
98: mov edi, 98
99: mov eax, 231
    syscall
    ud2

/* A thread as pthread_create makes one: its own stack, TLS through FS
   (the word at DATA + 0x1000), its tid at DATA + 0x20 (PARENT_SETTID)
   which it clears and wakes at exit (CHILD_CLEARTID); the main thread
   waits on that futex, then checks what the thread stored. The thread
   also leaves a lock it holds on its robust list (head at DATA + 0x500,
   one entry at 0x520 with the lock word 8 bytes in): its exit marks it
   FUTEX_OWNER_DIED. */
    .global task_test_thread
task_test_thread:
    mov rbx, {data}
    mov rdi, {thread_flags}
    lea rsi, [rbx + 0x2000]
    lea rdx, [rbx + 0x20]
    lea r10, [rbx + 0x20]
    lea r8, [rbx + 0x1000]
    mov eax, 56
    syscall
    test rax, rax
    jz 1f
    js 90f
    mov r12, rax
    mov eax, dword ptr [rbx + 0x20]
    test eax, eax
    jz 3f
    cmp eax, r12d
    jne 91f
3:
    mov eax, dword ptr [rbx + 0x20]
    test eax, eax
    jz 4f
    lea rdi, [rbx + 0x20]
    xor esi, esi
    mov edx, eax
    xor r10d, r10d
    mov eax, 202
    syscall
    jmp 3b
4:
    cmp qword ptr [rbx + 0x30], 0x1234
    jne 92f
    cmp qword ptr [rbx + 0x38], r12
    jne 93f
    cmp qword ptr [rbx + 0x40], 0
    jne 94f
    cmp dword ptr [rbx + 0x528], 0x40000000
    jne 95f
    xor edi, edi
    jmp 99f
1:
    mov rax, qword ptr fs:[0]
    mov qword ptr [rbx + 0x30], rax
    mov eax, 186
    syscall
    mov qword ptr [rbx + 0x38], rax
    lea rax, [rbx + 0x2000]
    cmp rsp, rax
    setne al
    movzx eax, al
    mov qword ptr [rbx + 0x40], rax
    lea rax, [rbx + 0x520]
    mov qword ptr [rbx + 0x500], rax
    mov qword ptr [rbx + 0x508], 8
    mov qword ptr [rbx + 0x510], 0
    lea rax, [rbx + 0x500]
    mov qword ptr [rbx + 0x520], rax
    mov rax, qword ptr [rbx + 0x38]
    mov dword ptr [rbx + 0x528], eax
    lea rdi, [rbx + 0x500]
    mov esi, 24
    mov eax, 273
    syscall
    test rax, rax
    jnz 6f
    xor edi, edi
    mov eax, 60
    syscall
    ud2
6:
    mov edi, 2
    mov eax, 60
    syscall
    ud2
90: mov edi, 90
    jmp 99f
91: mov edi, 91
    jmp 99f
92: mov edi, 92
    jmp 99f
93: mov edi, 93
    jmp 99f
94: mov edi, 94
    jmp 99f
95: mov edi, 95
99: mov eax, 231
    syscall
    ud2

/* Orphans: a shared anonymous page, then a child A that forks a
   grandchild G and ends as soon as G has recorded its parent (A). G waits
   until A has gone, records its new parent, and sets a flag in the shared page,
   where the parent (which reaped A) finds all three. */
    .global task_test_orphan
task_test_orphan:
    xor edi, edi
    mov esi, 4096
    mov edx, 3
    mov r10d, 0x21
    mov r8, -1
    xor r9d, r9d
    mov eax, 9
    syscall
    cmp rax, -4096
    ja 90f
    mov rbx, rax
    mov eax, 39
    syscall
    mov r15, rax
    mov eax, 57
    syscall
    test rax, rax
    jz 1f
    js 90f
    mov r12, rax
    mov rdi, r12
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    mov eax, 61
    syscall
    cmp rax, r12
    jne 91f
2:
    cmp qword ptr [rbx], 0
    je 2b
    cmp qword ptr [rbx + 8], r12
    jne 92f
    xor eax, eax
    cmp r15, 1
    cmove rax, r15
    cmp qword ptr [rbx + 16], rax
    jne 93f
    xor edi, edi
    jmp 99f
1:
    mov eax, 39
    syscall
    mov r14, rax
    mov eax, 57
    syscall
    test rax, rax
    jz 3f
5:
    cmp qword ptr [rbx + 24], 0
    je 5b
    xor edi, edi
    jmp 99f
3:
    mov eax, 110
    syscall
    mov qword ptr [rbx + 8], rax
    mov qword ptr [rbx + 24], 1
4:
    mov eax, 110
    syscall
    cmp rax, r14
    je 4b
    mov qword ptr [rbx + 16], rax
    mov qword ptr [rbx], 1
    xor edi, edi
    jmp 99f
90: mov edi, 90
    jmp 99f
91: mov edi, 91
    jmp 99f
92: mov edi, 92
    jmp 99f
93: mov edi, 93
99: mov eax, 231
    syscall
    ud2

/* vfork; the child marks DATA + 0x48 in the shared memory and execs the
   path at DATA + 0x200 with the argv at DATA + 0x300 (exit_group with the
   errno if that fails); the parent, resumed only then, checks the mark,
   reaps the child and exits with the child's exit status. */
    .global task_test_vfork_exec
task_test_vfork_exec:
    mov rbx, {data}
    mov eax, 58
    syscall
    test rax, rax
    jz 1f
    js 90f
    mov r12, rax
    cmp qword ptr [rbx + 0x48], 1
    jne 91f
    mov rdi, r12
    mov rsi, rbx
    xor edx, edx
    xor r10d, r10d
    mov eax, 61
    syscall
    cmp rax, r12
    jne 92f
    mov edi, dword ptr [rbx]
    test edi, 0x7f
    jnz 93f
    shr edi, 8
    jmp 99f
1:
    mov qword ptr [rbx + 0x48], 1
    lea rdi, [rbx + 0x200]
    lea rsi, [rbx + 0x300]
    xor edx, edx
    mov eax, 59
    syscall
    mov rdi, rax
    neg rdi
    jmp 99f
90: mov edi, 90
    jmp 99f
91: mov edi, 91
    jmp 99f
92: mov edi, 92
    jmp 99f
93: mov edi, 93
99: mov eax, 231
    syscall
    ud2

/* A second thread execs the path at DATA + 0x200 while the main thread
   spins; exit_group with the errno if exec fails. */
    .global task_test_thread_exec
task_test_thread_exec:
    mov rbx, {data}
    mov rdi, {plain_thread}
    lea rsi, [rbx + 0x3000]
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    mov eax, 56
    syscall
    test rax, rax
    jz 1f
    js 90f
2:
    pause
    jmp 2b
1:
    lea rdi, [rbx + 0x200]
    lea rsi, [rbx + 0x300]
    xor edx, edx
    mov eax, 59
    syscall
    mov rdi, rax
    neg rdi
    jmp 99f
90: mov edi, 90
99: mov eax, 231
    syscall
    ud2

    .balign 16
    .global task_test_end
task_test_end:
"#,
    data = const DATA,
    thread_flags = const THREAD_FLAGS,
    plain_thread = const PLAIN_THREAD,
);

unsafe extern "C" {
    static task_test_start: u8;
    static task_test_end: u8;
    static task_test_fork: u8;
    static task_test_thread: u8;
    static task_test_orphan: u8;
    static task_test_vfork_exec: u8;
    static task_test_thread_exec: u8;
}

/// A program's address once the blob is mapped at [`CODE`].
fn at(label: *const u8) -> u64 {
    CODE + (label as u64 - (&raw const task_test_start) as u64)
}

fn blob() -> &'static [u8] {
    let start = &raw const task_test_start;
    let len = (&raw const task_test_end) as usize - start as usize;
    // SAFETY: the assembly above puts every program between the two
    // symbols, in one read-only section of the kernel image.
    unsafe { core::slice::from_raw_parts(start, len) }
}

/// A Linux process (a root of its own tree) and a memory with the
/// programs, the data pages and a stack.
fn setup() -> TestResult<(Arc<Process>, Arc<Memory>)> {
    let format = Format::Elf { bits: Bits::B64, arch: Arch::X86_64 };
    let process = Arc::new(Process::new_linux(process::alloc_pid(), 0, format, false));
    let memory = Memory::new(Layout::default()).map_err(|_| "no memory for a memory map")?;
    let code = blob();
    let len = poly_vm::page_up(code.len() as u64).expect("small");
    let ok = memory.map_anonymous(Placement::FixedNoReplace(CODE), len, Prot::RW).is_ok()
        && memory.write(CODE, code).is_ok()
        && memory.protect(CODE, len, Prot::RX).is_ok()
        && memory.map_anonymous(Placement::FixedNoReplace(DATA), DATA_LEN, Prot::RW).is_ok()
        && memory.setup_stack(64 << 10, Prot::RW).is_ok();
    if !ok {
        return Err("could not lay out the test memory");
    }
    Ok((process, Arc::new(memory)))
}

/// Start the main thread of `process` at `program` with `hook`.
fn start(process: &Arc<Process>, memory: &Arc<Memory>, program: *const u8, hook: user::SyscallHook) -> TestResult {
    let entry = Entry { rip: at(program), rsp: memory.layout().stack_top - 64, args: [0; 3] };
    let tid = user::spawn_in(Arc::clone(process), Arc::clone(memory), entry, hook).map_err(|_| "spawn failed")?;
    let _ = crate::thread::detach(tid);
    Ok(())
}

fn wait(process: &Process) -> TestResult<ExitStatus> {
    process.wait_until(time::now_ns() + DEADLINE_NS).ok_or("the test process did not end in time")
}

fn expect_exit(status: ExitStatus, code: u8, what: &'static str) -> TestResult {
    if status == ExitStatus::Exited(code) {
        Ok(())
    } else {
        crate::kprintln!("linux task test: {what}: {status}");
        Err(what)
    }
}

/// Wait until every user frame the test took is free again: the threads'
/// memories go when the scheduler reaps them, a moment after they end.
fn frames_back_to(before: u64) -> TestResult {
    let deadline = time::now_ns() + DEADLINE_NS;
    while user::user_frames() != before {
        if time::now_ns() > deadline {
            crate::kprintln!("linux task test: {} user frames, {before} before", user::user_frames());
            return Err("user frames were not freed after the processes ended");
        }
        crate::thread::sleep(core::time::Duration::from_millis(2));
    }
    Ok(())
}

/// Run `program` as a fresh process with the default hook; returns its
/// exit status once it and its memory are gone (and checks the frames).
fn run(program: *const u8, prepare: impl FnOnce(&Memory) -> TestResult) -> TestResult<ExitStatus> {
    let before = user::user_frames();
    let status = {
        let (process, memory) = setup()?;
        prepare(&memory)?;
        start(&process, &memory, program, user::DEFAULT_HOOK)?;
        wait(&process)?
    };
    frames_back_to(before)?;
    Ok(status)
}

kernel_test! {
    fn fork_copies_memory_on_write_and_wait4_reaps_the_child() -> TestResult {
        let status = run(&raw const task_test_fork, |_| Ok(()))?;
        expect_exit(status, 0, "fork / wait4 checks failed (see the program for the number)")
    }
}

kernel_test! {
    fn threads_have_tls_clear_child_tid_wakes_the_joiner_and_robust_locks_are_released() -> TestResult {
        let status = run(&raw const task_test_thread, |m| {
            m.write(DATA + 0x1000, &0x1234u64.to_le_bytes()).map_err(|_| "writing the TLS word failed")
        })?;
        expect_exit(status, 0, "thread checks failed (see the program for the number)")
    }
}

kernel_test! {
    fn orphans_are_adopted_and_shared_anonymous_memory_stays_shared() -> TestResult {
        let status = run(&raw const task_test_orphan, |_| Ok(()))?;
        expect_exit(status, 0, "orphan checks failed (see the program for the number)")
    }
}

/// Put `path` and `argv = [path, NULL]` where the exec programs find them.
fn exec_args(memory: &Memory, path: &[u8]) -> TestResult {
    let mut p = path.to_vec();
    p.push(0);
    let mut argv = [0u8; 16];
    argv[..8].copy_from_slice(&(DATA + 0x200).to_le_bytes());
    let ok = memory.write(DATA + 0x200, &p).is_ok() && memory.write(DATA + 0x300, &argv).is_ok();
    if ok { Ok(()) } else { Err("writing the exec arguments failed") }
}

kernel_test! {
    fn vfork_waits_for_exec_and_scripts_run_their_interpreter(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(fs::TEST_INITRD_FLAG) {
            return Ok(());
        }
        let status = run(&raw const task_test_vfork_exec, |m| exec_args(m, b"/bin/hello"))?;
        expect_exit(status, 0, "vfork + exec of /bin/hello failed")?;
        let status = run(&raw const task_test_vfork_exec, |m| exec_args(m, b"/nope"))?;
        expect_exit(status, 2, "exec of a missing file was not ENOENT")?;
        // A script whose interpreter is /bin/hello.
        const SCRIPT: &[u8] = b"/tmp/linux-task-script";
        let files = fs::Files::new();
        let fd = files
            .open(SCRIPT, OpenFlags::CREAT | OpenFlags::WRONLY | OpenFlags::TRUNC, 0o755)
            .map_err(|e| e.name())?;
        let written = files.write(fd, b"#!/bin/hello -x\n");
        let _ = files.close(fd);
        written.map_err(|e| e.name())?;
        let status = run(&raw const task_test_vfork_exec, |m| exec_args(m, SCRIPT));
        let _ = files.unlinkat(fs::AT_FDCWD, SCRIPT, 0);
        expect_exit(status?, 0, "a #! script did not run its interpreter")
    }
}

/// What [`exec_hook`] saw after a successful execve: one bit per check.
static EXEC_CHECKS: AtomicU64 = AtomicU64::new(0);

/// The default hook, plus checks right after a successful execve.
fn exec_hook(process: &Process, regs: &mut SyscallRegs) {
    let number = regs.rax;
    user::DEFAULT_HOOK(process, regs);
    if number == poly_personality_linux::nr::EXECVE && regs.rax == 0 {
        let memory = user::current_memory();
        let checks = [
            process.thread_count() == 1,
            process.files.file(5).is_err(),
            process.files.file(6).is_ok(),
            user::current_tid() == Some(process.pid),
            memory.is_some_and(|m| m.regions().iter().all(|v| !v.contains(DATA))),
            user::fs_base() == 0,
        ];
        let bits = checks.iter().enumerate().fold(0, |acc, (i, &ok)| acc | (u64::from(ok) << i));
        EXEC_CHECKS.store(bits | 1 << 63, Ordering::SeqCst);
    }
}

kernel_test! {
    fn execve_replaces_the_image_ends_other_threads_and_closes_cloexec_files(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(fs::TEST_INITRD_FLAG) {
            return Ok(());
        }
        EXEC_CHECKS.store(0, Ordering::SeqCst);
        let before = user::user_frames();
        let status = {
            let (process, memory) = setup()?;
            exec_args(&memory, b"/bin/hello")?;
            let files = &process.files;
            let cloexec = files.open(b"/dev/null", OpenFlags::RDONLY | OpenFlags::CLOEXEC, 0).map_err(|e| e.name())?;
            let kept = files.open(b"/dev/null", OpenFlags::RDONLY, 0).map_err(|e| e.name())?;
            files.dup2(cloexec, 5).map_err(|e| e.name())?;
            files.set_cloexec(5, true).map_err(|e| e.name())?;
            files.dup2(kept, 6).map_err(|e| e.name())?;
            let _ = files.close(cloexec);
            let _ = files.close(kept);
            start(&process, &memory, &raw const task_test_thread_exec, exec_hook)?;
            wait(&process)?
        };
        frames_back_to(before)?;
        expect_exit(status, 0, "the exec'd /bin/hello did not exit with 0")?;
        let checks = EXEC_CHECKS.load(Ordering::SeqCst);
        if checks != (1 << 63 | 0x3f) {
            crate::kprintln!("linux task test: exec checks {checks:#x}");
            return Err("after execve: threads, close-on-exec, the pid, the memory or the FS base were not as expected");
        }
        Ok(())
    }
}
