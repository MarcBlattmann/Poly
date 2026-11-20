//! libsystem_kernel: the system call wrappers (ADR 0258).
//!
//! Each BSD wrapper is a few instructions, as the spec sheets give the
//! convention: `rax` = 0x2000000 | number, the fourth argument moved from
//! `rcx` to `r10`, `syscall`; with the carry flag clear `rax` is the
//! result, with it set `rax` is the errno, which goes to `errno` (a
//! thread-local variable) and the wrapper returns -1. A Mach trap is
//! 0x1000000 | number and returns its `kern_return_t`. Arguments past the
//! sixth stay where the caller put them, on the stack.
//!
//! The wrappers cover the calls that have spec sheets (`specs/macos/bsd`,
//! `specs/macos/mach`), under the names the SDK exports them by. The
//! `$NOCANCEL` forms are the plain calls (no thread can be cancelled
//! yet); the `$INODE64` forms are the 64-bit `stat` calls. The functions
//! the SDK declares over a trap with the same arguments
//! (`mach_port_allocate` …) are the trap itself; `semaphore_timedwait*`
//! split their `mach_timespec_t`; `posix_spawn` maps its six C arguments
//! to the call's five and returns the errno (`__posix_spawn` is the raw
//! call).

/// A BSD wrapper per (name, number), then the error tail.
macro_rules! bsd {
    ($($name:literal = $num:literal),* $(,)?) => {
        core::arch::global_asm!(
            $(
                concat!(".globl \"_", $name, "\""),
                ".p2align 4",
                concat!("\"_", $name, "\":"),
                concat!("    mov eax, 0x2000000 + ", $num),
                "    mov r10, rcx",
                "    syscall",
                "    jae 2f",
                "    mov rdi, rax",
                "    jmp _poly_cerror",
                "2:",
                "    ret",
            )*
        );
    };
}

/// A Mach trap wrapper per trap, under one or more names.
macro_rules! mach {
    ($($num:literal => [$($name:literal),+]),* $(,)?) => {
        core::arch::global_asm!(
            $(
                $(concat!(".globl \"_", $name, "\""),)+
                ".p2align 4",
                $(concat!("\"_", $name, "\":"),)+
                concat!("    mov eax, 0x1000000 + ", $num),
                "    mov r10, rcx",
                "    syscall",
                "    ret",
            )*
        );
    };
}

bsd! {
    "_exit" = 1, "__exit" = 1,
    "read" = 3, "read$NOCANCEL" = 3,
    "write" = 4, "write$NOCANCEL" = 4, "__write_nocancel" = 4,
    "open" = 5, "open$NOCANCEL" = 5, "__open_nocancel" = 5,
    "close" = 6, "close$NOCANCEL" = 6,
    "wait4" = 7,
    "chdir" = 12,
    "getpid" = 20,
    "kill" = 37,
    "sigprocmask" = 48,
    "sigpending" = 52,
    "ioctl" = 54,
    "execve" = 59,
    "msync" = 65,
    "munmap" = 73,
    "mprotect" = 74,
    "madvise" = 75,
    "mincore" = 78,
    "dup2" = 90,
    "fcntl" = 92,
    "sigsuspend" = 111,
    "rename" = 128,
    "csops" = 169,
    "waitid" = 173,
    "mmap" = 197,
    "__sysctl" = 202,
    "mlock" = 203,
    "getattrlist" = 220,
    "setattrlist" = 221,
    "fgetattrlist" = 228,
    "getxattr" = 234,
    "fgetxattr" = 235,
    "setxattr" = 236,
    "fsetxattr" = 237,
    "removexattr" = 238,
    "fremovexattr" = 239,
    "listxattr" = 240,
    "flistxattr" = 241,
    "__posix_spawn" = 244,
    "minherit" = 250,
    "__sysctlbyname" = 274,
    "__shared_region_check_np" = 294,
    "__psynch_mutexwait" = 301,
    "__psynch_mutexdrop" = 302,
    "__psynch_cvbroad" = 303,
    "__psynch_cvsignal" = 304,
    "__psynch_cvwait" = 305,
    "issetugid" = 327,
    "__proc_info" = 336,
    "stat$INODE64" = 338, "stat64" = 338,
    "fstat$INODE64" = 339, "fstat64" = 339,
    "lstat$INODE64" = 340, "lstat64" = 340,
    "__getdirentries64" = 344,
    "__bsdthread_create" = 360,
    "kqueue" = 362,
    "kevent" = 363,
    "__bsdthread_register" = 366,
    "kevent64" = 369,
    "__thread_selfid" = 372,
    "getattrlistbulk" = 461,
    "clonefileat" = 462,
    "renameat" = 465,
    "fstatat$INODE64" = 470, "fstatat64" = 470,
    "renameatx_np" = 488,
    "getentropy" = 500,
    "__ulock_wait" = 515,
    "__ulock_wake" = 516,
    "fclonefileat" = 517,
    "__ulock_wait2" = 544,
}

mach! {
    10 => ["_kernelrpc_mach_vm_allocate_trap", "mach_vm_allocate", "vm_allocate"],
    12 => ["_kernelrpc_mach_vm_deallocate_trap", "mach_vm_deallocate", "vm_deallocate"],
    14 => ["_kernelrpc_mach_vm_protect_trap", "mach_vm_protect", "vm_protect"],
    16 => ["_kernelrpc_mach_port_allocate_trap", "mach_port_allocate"],
    18 => ["_kernelrpc_mach_port_deallocate_trap", "mach_port_deallocate"],
    19 => ["_kernelrpc_mach_port_mod_refs_trap", "mach_port_mod_refs"],
    21 => ["_kernelrpc_mach_port_insert_right_trap", "mach_port_insert_right"],
    26 => ["mach_reply_port"],
    27 => ["thread_self_trap", "mach_thread_self"],
    28 => ["task_self_trap", "mach_task_self"],
    29 => ["host_self_trap", "mach_host_self"],
    31 => ["mach_msg_trap", "mach_msg"],
    33 => ["semaphore_signal_trap", "semaphore_signal"],
    34 => ["semaphore_signal_all_trap", "semaphore_signal_all"],
    35 => ["semaphore_signal_thread_trap", "semaphore_signal_thread"],
    36 => ["semaphore_wait_trap", "semaphore_wait"],
    37 => ["semaphore_wait_signal_trap", "semaphore_wait_signal"],
    38 => ["semaphore_timedwait_trap"],
    39 => ["semaphore_timedwait_signal_trap"],
    76 => ["_kernelrpc_mach_port_type_trap", "mach_port_type"],
    89 => ["mach_timebase_info_trap", "mach_timebase_info"],
}

core::arch::global_asm!(
    // posix_spawn(pid, path, file_actions, attrp, argv, envp): the C call
    // has six arguments, BSD call 244 five (pid, path, descriptor, argv,
    // envp; specs/macos/bsd/posix_spawn.yaml). The descriptor of file
    // actions and attributes is 0: v0 has neither (a non-empty one is
    // EINVAL). The result is the errno itself, as POSIX has it, not -1.
    ".globl _posix_spawn",
    ".p2align 4",
    "_posix_spawn:",
    "    test rdx, rdx",
    "    je 4f",
    "    cmp qword ptr [rdx], 0",
    "    jne 6f",
    "4:",
    "    test rcx, rcx",
    "    je 5f",
    "    cmp qword ptr [rcx], 0",
    "    jne 6f",
    "5:",
    "    xor edx, edx",
    "    mov r10, r8",
    "    mov r8, r9",
    "    mov eax, 0x2000000 + 244",
    "    syscall",
    "    jb 7f",
    "    xor eax, eax",
    "7:",
    "    ret",
    "6:",
    "    mov eax, 22",
    "    ret",
    // pipe(fds): the two descriptors come back in rax and rdx.
    ".globl _pipe",
    ".p2align 4",
    "_pipe:",
    "    push rdi",
    "    mov eax, 0x2000000 + 42",
    "    syscall",
    "    pop rdi",
    "    jae 2f",
    "    mov rdi, rax",
    "    jmp _poly_cerror",
    "2:",
    "    mov dword ptr [rdi], eax",
    "    mov dword ptr [rdi + 4], edx",
    "    xor eax, eax",
    "    ret",
    // semaphore_timedwait(sem, mach_timespec_t {sec, nsec}): the struct
    // comes in one register; the trap takes (sem, sec, nsec).
    ".globl _semaphore_timedwait",
    ".p2align 4",
    "_semaphore_timedwait:",
    "    mov edx, esi",
    "    shr rsi, 32",
    "    xchg esi, edx",
    "    mov eax, 0x1000000 + 38",
    "    syscall",
    "    ret",
    // semaphore_timedwait_signal(wait, signal, mach_timespec_t).
    ".globl _semaphore_timedwait_signal",
    ".p2align 4",
    "_semaphore_timedwait_signal:",
    "    mov ecx, edx",
    "    shr rdx, 32",
    "    xchg edx, ecx",
    "    mov r10, rcx",
    "    mov eax, 0x1000000 + 39",
    "    syscall",
    "    ret",
);

// errno: one `int` per thread, a Mach-O thread-local variable. dyld points
// the descriptor's thunk at its own; `_poly_tlv_unbound` only fills the
// word until then.
core::arch::global_asm!(
    ".section __DATA,__thread_vars,thread_local_variables",
    ".p2align 3",
    "_poly_errno:",
    "    .quad _poly_tlv_unbound",
    "    .quad 0",
    "    .quad _poly_errno_init",
    ".tbss _poly_errno_init, 4, 2",
    ".text",
    ".p2align 4",
    "_poly_tlv_unbound:",
    "    ud2",
    ".globl ___error",
    ".p2align 4",
    "___error:",
    "    movq _poly_errno@TLVP(%rip), %rdi",
    "    jmpq *(%rdi)",
    options(att_syntax),
);

unsafe extern "C" {
    pub fn __error() -> *mut i32;
}

/// The error tail of every BSD wrapper (the errno moved from `rax` to the
/// first argument): `errno = errno`, return -1.
#[unsafe(no_mangle)]
extern "C" fn poly_cerror(errno: u64) -> i64 {
    crate::set_errno(errno as i32);
    -1
}

/// `mach_task_self_`: the task's own port name, read once at start.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut mach_task_self_: u32 = 0;

unsafe extern "C" {
    fn task_self_trap() -> u32;
}

extern "C" fn init(_: i32, _: *const *const u8, _: *const *const u8, _: *const *const u8, _: *const u8) {
    // SAFETY: a trap without arguments; initializers run before any thread
    // other than the main one exists.
    unsafe { mach_task_self_ = task_self_trap() };
}

#[used]
#[unsafe(link_section = "__DATA,__mod_init_func,mod_init_funcs")]
static INIT: extern "C" fn(i32, *const *const u8, *const *const u8, *const *const u8, *const u8) = init;
