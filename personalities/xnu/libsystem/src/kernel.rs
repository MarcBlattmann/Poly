//! libsystem_kernel: the system call wrappers (ADR 0258).
//!
//! Each BSD wrapper is a few instructions, as the spec sheets give the
//! convention: `rax` = 0x2000000 | number, the fourth argument moved from
//! `rcx` to `r10`, `syscall`; with the carry flag clear `rax` is the
//! result, with it set `rax` is the errno, which goes to `errno` (a
//! thread-local variable) and the wrapper returns -1. A Mach trap is
//! 0x1000000 | number and returns its `kern_return_t`. Arguments past the
//! sixth stay where the caller put them, on the stack. On arm64 (ADR
//! 0257) the number goes in `x16` (a Mach trap's negated), `svc #0x80`,
//! and the carry flag means `x0` is the errno.
//!
//! The wrappers cover the calls that have spec sheets (`specs/macos/bsd`,
//! `specs/macos/mach`), under the names the SDK exports them by. The
//! `$NOCANCEL` forms are the plain calls (no thread can be cancelled
//! yet); the `$INODE64` forms are the 64-bit `stat` calls. The functions
//! the SDK declares over a trap with the same arguments
//! (`mach_port_allocate` …) are the trap itself; `semaphore_timedwait*`
//! split their `mach_timespec_t`.

/// A BSD wrapper per (name, number), then the error tail.
#[cfg(target_arch = "x86_64")]
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

/// arm64 (ADR 0257): `svc #0x80` with the number in `x16`; the carry flag
/// set means `x0` is the errno.
#[cfg(target_arch = "aarch64")]
macro_rules! bsd {
    ($($name:literal = $num:literal),* $(,)?) => {
        core::arch::global_asm!(
            $(
                concat!(".globl \"_", $name, "\""),
                ".p2align 2",
                concat!("\"_", $name, "\":"),
                concat!("    mov x16, #", $num),
                "    svc #0x80",
                "    b.cc 2f",
                "    b _poly_cerror",
                "2:",
                "    ret",
            )*
        );
    };
}

/// A Mach trap wrapper per trap, under one or more names.
#[cfg(target_arch = "x86_64")]
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

/// arm64: a Mach trap is `x16` = minus its number.
#[cfg(target_arch = "aarch64")]
macro_rules! mach {
    ($($num:literal => [$($name:literal),+]),* $(,)?) => {
        core::arch::global_asm!(
            $(
                $(concat!(".globl \"_", $name, "\""),)+
                ".p2align 2",
                $(concat!("\"_", $name, "\":"),)+
                concat!("    mov x16, #-", $num),
                "    svc #0x80",
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
    "posix_spawn" = 244, "__posix_spawn" = 244,
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
    "stat64" = 338,
    "fstat64" = 339,
    "lstat64" = 340,
    "__getdirentries64" = 344,
    "__bsdthread_create" = 360,
    "__bsdthread_terminate" = 361,
    "kqueue" = 362,
    "kevent" = 363,
    "__bsdthread_register" = 366,
    "kevent64" = 369,
    "__thread_selfid" = 372,
    "getattrlistbulk" = 461,
    "clonefileat" = 462,
    "renameat" = 465,
    "fstatat64" = 470,
    "renameatx_np" = 488,
    "getentropy" = 500,
    "__ulock_wait" = 515,
    "__ulock_wake" = 516,
    "fclonefileat" = 517,
    "__ulock_wait2" = 544,
}

// More wrappers for SDK-linked command-line programs (ADR 0257 addendum):
// call numbers from the SDK's <sys/syscall.h>; the kernel answers ENOSYS
// where it does not have the call yet.
bsd! {
    "unlink" = 10,
    "chmod" = 15,
    "getuid" = 24,
    "geteuid" = 25,
    "access" = 33,
    "getppid" = 39,
    "dup" = 41,
    "getegid" = 43,
    "__sigaction" = 46,
    "getgid" = 47,
    "symlink" = 57,
    "readlink" = 58,
    "umask" = 60,
    "fsync" = 95,
    "getrusage" = 117,
    "fchown" = 123,
    "fchmod" = 124,
    "flock" = 131,
    "mkdir" = 136,
    "rmdir" = 137,
    "utimes" = 138,
    "futimes" = 139,
    "gethostuuid" = 142,
    "pread" = 153,
    "pwrite" = 154,
    "lseek" = 199,
    "truncate" = 200,
    "ftruncate" = 201,
    "fsctl" = 242,
    "statfs64" = 345,
    "fstatfs64" = 346,
}

#[cfg(target_arch = "x86_64")]
bsd! {
    "statfs$INODE64" = 345,
    "fstatfs$INODE64" = 346,
}

#[cfg(target_arch = "aarch64")]
bsd! {
    "statfs" = 345,
    "fstatfs" = 346,
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

// The 64-bit `stat` calls: `$INODE64` names on x86-64, plain names on
// arm64, where `stat` always has 64-bit inodes (the SDK's `.tbd`).
#[cfg(target_arch = "x86_64")]
bsd! {
    "stat$INODE64" = 338,
    "fstat$INODE64" = 339,
    "lstat$INODE64" = 340,
    "fstatat$INODE64" = 470,
}

#[cfg(target_arch = "aarch64")]
bsd! {
    "stat" = 338,
    "fstat" = 339,
    "lstat" = 340,
    "fstatat" = 470,
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
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

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    // pipe(fds): the two descriptors come back in x0 and x1 (the pipe
    // sheet).
    ".globl _pipe",
    ".p2align 2",
    "_pipe:",
    "    mov x9, x0",
    "    mov x16, #42",
    "    svc #0x80",
    "    b.cc 2f",
    "    b _poly_cerror",
    "2:",
    "    stp w0, w1, [x9]",
    "    mov w0, #0",
    "    ret",
    // semaphore_timedwait(sem, mach_timespec_t {sec, nsec}): the struct
    // comes in one register; the trap takes (sem, sec, nsec).
    ".globl _semaphore_timedwait",
    ".p2align 2",
    "_semaphore_timedwait:",
    "    lsr x2, x1, #32",
    "    mov w1, w1",
    "    mov x16, #-38",
    "    svc #0x80",
    "    ret",
    // semaphore_timedwait_signal(wait, signal, mach_timespec_t).
    ".globl _semaphore_timedwait_signal",
    ".p2align 2",
    "_semaphore_timedwait_signal:",
    "    lsr x3, x2, #32",
    "    mov w2, w2",
    "    mov x16, #-39",
    "    svc #0x80",
    "    ret",
);

// errno: one `int` per thread, a Mach-O thread-local variable. dyld points
// the descriptor's thunk at its own; `_poly_tlv_unbound` only fills the
// word until then.
#[cfg(target_arch = "x86_64")]
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

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".section __DATA,__thread_vars,thread_local_variables",
    ".p2align 3",
    "_poly_errno:",
    "    .quad _poly_tlv_unbound",
    "    .quad 0",
    "    .quad _poly_errno_init",
    ".tbss _poly_errno_init, 4, 2",
    ".text",
    ".p2align 2",
    "_poly_tlv_unbound:",
    "    brk #1",
    // The descriptor's address in x0, then its thunk, which returns the
    // variable's address in x0 (and changes no other register).
    ".globl ___error",
    ".p2align 2",
    "___error:",
    "    adrp x0, _poly_errno@TLVPPAGE",
    "    ldr x0, [x0, _poly_errno@TLVPPAGEOFF]",
    "    ldr x16, [x0]",
    "    br x16",
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
