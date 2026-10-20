//! In-kernel tests of credentials and file permissions (ADR 0045): small
//! programs in ring 3 that change their ids and capabilities through real
//! system calls and then meet the VFS's permission checks; a set-user-ID
//! program run with execve; the books (modes, owners) checked by the
//! kernel afterwards.
//!
//! Each program exits with 0 when every check it makes passed, else with
//! the number of the first failed one (90 and up). Files are opened and
//! removed through two test system calls of [`hook`] (`TEST_OPEN`,
//! `TEST_UNLINK`), which run `fs::Files` on the calling thread, with its
//! credentials, until the file system calls have system call handlers of
//! their own.

use crate::fs::{self, Files, OpenFlags};
use crate::kernel_test;
use crate::process::{self, ExitStatus, Process};
use crate::time;
use crate::user::memory::{Layout, Placement, Prot};
use crate::user::{self, Entry, Memory};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_abi::SyscallRegs;
use poly_binfmt::{Arch, Bits, Format};
use poly_personality_linux::task::clone::{CLONE_FILES, CLONE_FS, CLONE_SIGHAND, CLONE_THREAD, CLONE_VM};

type TestResult<T = ()> = Result<T, &'static str>;

/// Where the programs are mapped (read, execute).
const CODE: u64 = 0x40_0000;
/// Four read-write pages of data, zeroed; the paths go here.
const DATA: u64 = 0x60_0000;
const DATA_LEN: u64 = 0x4000;
/// Generous: a loaded host makes the emulator slow.
const DEADLINE_NS: u64 = 60 * time::NANOS_PER_SEC;

/// Test system call: `open(rdi = path, rsi = flags, rdx = mode)` through
/// the process's `Files`, as the calling thread.
const TEST_OPEN: u64 = 0x7000_0001;
/// Test system call: `unlink(rdi = path)`.
const TEST_UNLINK: u64 = 0x7000_0002;

/// Paths in the data pages: `PATH_A` at `DATA + 0x100`, `PATH_B` at
/// `+ 0x180`, `PATH_C` at `+ 0x200`, the program to exec at `+ 0x280`
/// with `argv = [it, NULL]` at `+ 0x300`.
const A: u64 = 0x100;
const B: u64 = 0x180;
const C: u64 = 0x200;
const EXEC: u64 = 0x280;

/// A thread without TLS or tid pointers.
const PLAIN_THREAD: u64 = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD;

global_asm!(
    r#"
    .section .rodata.linux_cred_test_programs, "a", @progbits
    .balign 16
    .global cred_test_start
cred_test_start:

/* Root opens PATH_A (0600, root's); after setresuid(1000, 1000, 1000)
   open and access are EACCES, setuid(0) is EPERM, every capability is
   gone, and umask works. */
    .global cred_test_drop
cred_test_drop:
    mov rbx, {data}
    lea rdi, [rbx + {a}]
    xor esi, esi
    xor edx, edx
    mov rax, {test_open}
    syscall
    test rax, rax
    js 90f
    mov rdi, rax
    mov eax, 3
    syscall
    mov edi, 1000
    mov esi, 1000
    mov edx, 1000
    mov eax, 117
    syscall
    test rax, rax
    jnz 91f
    mov rdi, rbx
    lea rsi, [rbx + 4]
    lea rdx, [rbx + 8]
    mov eax, 118
    syscall
    test rax, rax
    jnz 92f
    cmp dword ptr [rbx], 1000
    jne 92f
    cmp dword ptr [rbx + 4], 1000
    jne 92f
    cmp dword ptr [rbx + 8], 1000
    jne 92f
    lea rdi, [rbx + {a}]
    xor esi, esi
    xor edx, edx
    mov rax, {test_open}
    syscall
    cmp rax, -13
    jne 93f
    lea rdi, [rbx + {a}]
    mov esi, 4
    mov eax, 21
    syscall
    cmp rax, -13
    jne 94f
    xor edi, edi
    mov eax, 105
    syscall
    cmp rax, -1
    jne 95f
    mov dword ptr [rbx + 0x400], 0x20080522
    mov dword ptr [rbx + 0x404], 0
    lea rdi, [rbx + 0x400]
    lea rsi, [rbx + 0x408]
    mov eax, 125
    syscall
    test rax, rax
    jnz 96f
    mov eax, dword ptr [rbx + 0x408]
    or eax, dword ptr [rbx + 0x40c]
    or eax, dword ptr [rbx + 0x414]
    or eax, dword ptr [rbx + 0x418]
    test eax, eax
    jnz 96f
    mov edi, 63
    mov eax, 95
    syscall
    cmp rax, 18
    jne 97f
    mov edi, 18
    mov eax, 95
    syscall
    cmp rax, 63
    jne 97f
    xor edi, edi
    jmp 99f

/* Optionally no_new_privs (DATA + 0x800 non-zero), then
   setresuid(1000, 1000, 1000) and execve(EXEC): the program reports. */
    .global cred_test_suid
cred_test_suid:
    mov rbx, {data}
    cmp qword ptr [rbx + 0x800], 0
    je 1f
    mov edi, 38
    mov esi, 1
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    mov eax, 157
    syscall
    test rax, rax
    jnz 90f
1:
    mov edi, 1000
    mov esi, 1000
    mov edx, 1000
    mov eax, 117
    syscall
    test rax, rax
    jnz 91f
    lea rdi, [rbx + {exec}]
    lea rsi, [rbx + 0x300]
    xor edx, edx
    mov eax, 59
    syscall
    mov edi, 92
    jmp 99f

/* As root, the effective set emptied (permitted kept), then execve(EXEC):
   root gets its capabilities back. */
    .global cred_test_exec_root
cred_test_exec_root:
    mov rbx, {data}
    mov dword ptr [rbx + 0x400], 0x20080522
    mov dword ptr [rbx + 0x404], 0
    mov dword ptr [rbx + 0x408], 0
    mov dword ptr [rbx + 0x40c], 0xffffffff
    mov dword ptr [rbx + 0x410], 0
    mov dword ptr [rbx + 0x414], 0
    mov dword ptr [rbx + 0x418], 0x1ff
    mov dword ptr [rbx + 0x41c], 0
    lea rdi, [rbx + 0x400]
    lea rsi, [rbx + 0x408]
    mov eax, 126
    syscall
    test rax, rax
    jnz 90f
    lea rdi, [rbx + {a}]
    xor esi, esi
    xor edx, edx
    mov rax, {test_open}
    syscall
    test rax, rax
    jns 91f
    lea rdi, [rbx + {exec}]
    lea rsi, [rbx + 0x300]
    xor edx, edx
    mov eax, 59
    syscall
    mov edi, 92
    jmp 99f

/* PR_SET_KEEPCAPS, then every uid to 1000: the permitted set stays, the
   effective set is empty; raising CAP_DAC_OVERRIDE into it opens root's
   0600 PATH_A; then execve(EXEC) loses them all. */
    .global cred_test_keepcaps
cred_test_keepcaps:
    mov rbx, {data}
    mov edi, 8
    mov esi, 1
    mov eax, 157
    syscall
    test rax, rax
    jnz 90f
    mov edi, 1000
    mov esi, 1000
    mov edx, 1000
    mov eax, 117
    syscall
    test rax, rax
    jnz 91f
    mov edi, 7
    mov eax, 157
    syscall
    cmp rax, 1
    jne 92f
    mov dword ptr [rbx + 0x400], 0x20080522
    mov dword ptr [rbx + 0x404], 0
    lea rdi, [rbx + 0x400]
    lea rsi, [rbx + 0x408]
    mov eax, 125
    syscall
    test rax, rax
    jnz 93f
    cmp dword ptr [rbx + 0x408], 0
    jne 93f
    cmp dword ptr [rbx + 0x40c], 0xffffffff
    jne 93f
    lea rdi, [rbx + {a}]
    xor esi, esi
    xor edx, edx
    mov rax, {test_open}
    syscall
    cmp rax, -13
    jne 94f
    mov dword ptr [rbx + 0x408], 2
    lea rdi, [rbx + 0x400]
    lea rsi, [rbx + 0x408]
    mov eax, 126
    syscall
    test rax, rax
    jnz 95f
    lea rdi, [rbx + {a}]
    xor esi, esi
    xor edx, edx
    mov rax, {test_open}
    syscall
    test rax, rax
    js 96f
    lea rdi, [rbx + {exec}]
    lea rsi, [rbx + 0x300]
    xor edx, edx
    mov eax, 59
    syscall
    mov edi, 97
    jmp 99f

/* Root drops CAP_CHOWN from the bounding set (PR_CAPBSET_READ / DROP),
   and CAP_CHOWN, CAP_DAC_OVERRIDE and CAP_DAC_READ_SEARCH from its
   permitted and effective sets: PATH_B (0600, uid 1000's) cannot be
   opened, chown is EPERM, chmod still works (CAP_FOWNER), and the
   capabilities cannot come back. */
    .global cred_test_capdrop
cred_test_capdrop:
    mov rbx, {data}
    mov edi, 23
    xor esi, esi
    mov eax, 157
    syscall
    cmp rax, 1
    jne 90f
    mov edi, 24
    xor esi, esi
    mov eax, 157
    syscall
    test rax, rax
    jnz 91f
    mov edi, 23
    xor esi, esi
    mov eax, 157
    syscall
    test rax, rax
    jnz 92f
    mov dword ptr [rbx + 0x400], 0x20080522
    mov dword ptr [rbx + 0x404], 0
    mov dword ptr [rbx + 0x408], 0xfffffff8
    mov dword ptr [rbx + 0x40c], 0xfffffff8
    mov dword ptr [rbx + 0x410], 0
    mov dword ptr [rbx + 0x414], 0x1ff
    mov dword ptr [rbx + 0x418], 0x1ff
    mov dword ptr [rbx + 0x41c], 0
    lea rdi, [rbx + 0x400]
    lea rsi, [rbx + 0x408]
    mov eax, 126
    syscall
    test rax, rax
    jnz 93f
    lea rdi, [rbx + {b}]
    xor esi, esi
    xor edx, edx
    mov rax, {test_open}
    syscall
    cmp rax, -13
    jne 94f
    lea rdi, [rbx + {b}]
    xor esi, esi
    mov edx, -1
    mov eax, 92
    syscall
    cmp rax, -1
    jne 95f
    lea rdi, [rbx + {b}]
    mov esi, 420
    mov eax, 90
    syscall
    test rax, rax
    jnz 96f
    mov dword ptr [rbx + 0x408], 0xfffffffa
    lea rdi, [rbx + 0x400]
    lea rsi, [rbx + 0x408]
    mov eax, 126
    syscall
    cmp rax, -1
    jne 97f
    xor edi, edi
    jmp 99f

/* As uid 1001 in the sticky /tmp: PATH_C (uid 1000's) cannot be
   removed; a file of its own can. PATH_A is that file. */
    .global cred_test_sticky
cred_test_sticky:
    mov rbx, {data}
    mov edi, 1001
    mov esi, 1001
    mov edx, 1001
    mov eax, 117
    syscall
    test rax, rax
    jnz 90f
    lea rdi, [rbx + {c}]
    mov rax, {test_unlink}
    syscall
    cmp rax, -1
    jne 91f
    lea rdi, [rbx + {a}]
    mov esi, 0x41
    mov edx, 384
    mov rax, {test_open}
    syscall
    test rax, rax
    js 92f
    mov rdi, rax
    mov eax, 3
    syscall
    lea rdi, [rbx + {a}]
    mov rax, {test_unlink}
    syscall
    test rax, rax
    jnz 93f
    xor edi, edi
    jmp 99f

/* chown rules: groups [50], gids 1000, uids 1000. PATH_B is 06755 and
   uid 1000's: its group may become 50 (a group of the caller), not 60;
   its owner may not change; chmod 02755 keeps set-group-ID (the caller
   is in group 50); PATH_A (root's) cannot be chmod'ed; a write to
   PATH_B clears set-group-ID again (fstat: st_mode at offset 24). */
    .global cred_test_chown
cred_test_chown:
    mov rbx, {data}
    mov dword ptr [rbx + 0x500], 50
    mov edi, 1
    lea rsi, [rbx + 0x500]
    mov eax, 116
    syscall
    test rax, rax
    jnz 90f
    mov edi, 1000
    mov esi, 1000
    mov edx, 1000
    mov eax, 119
    syscall
    test rax, rax
    jnz 90f
    mov edi, 1000
    mov esi, 1000
    mov edx, 1000
    mov eax, 117
    syscall
    test rax, rax
    jnz 90f
    lea rdi, [rbx + {b}]
    mov esi, -1
    mov edx, 50
    mov eax, 92
    syscall
    test rax, rax
    jnz 91f
    lea rdi, [rbx + {b}]
    mov esi, 1001
    mov edx, -1
    mov eax, 92
    syscall
    cmp rax, -1
    jne 92f
    lea rdi, [rbx + {b}]
    mov esi, -1
    mov edx, 60
    mov eax, 92
    syscall
    cmp rax, -1
    jne 93f
    lea rdi, [rbx + {b}]
    mov esi, 1517
    mov eax, 90
    syscall
    test rax, rax
    jnz 94f
    lea rdi, [rbx + {a}]
    mov esi, 420
    mov eax, 90
    syscall
    cmp rax, -1
    jne 95f
    lea rdi, [rbx + {b}]
    mov esi, 1
    xor edx, edx
    mov rax, {test_open}
    syscall
    test rax, rax
    js 96f
    mov r12, rax
    mov rdi, r12
    lea rsi, [rbx + 0x900]
    mov eax, 5
    syscall
    test rax, rax
    jnz 96f
    mov eax, dword ptr [rbx + 0x918]
    and eax, 4095
    cmp eax, 1517
    jne 96f
    mov rdi, r12
    mov rsi, rbx
    mov edx, 1
    mov eax, 1
    syscall
    cmp rax, 1
    jne 97f
    mov rdi, r12
    lea rsi, [rbx + 0x900]
    mov eax, 5
    syscall
    mov eax, dword ptr [rbx + 0x918]
    and eax, 4095
    cmp eax, 493
    jne 97f
    xor edi, edi
    jmp 99f

/* Inheritance: groups [50] and gids 7 set, then fork. The child sees
   them, drops to uid 1000 and may not signal its root parent (kill 0:
   EPERM); it exits 0 or 80+. The parent stays root. Then a thread sets
   its own uids to 2000: the main thread's stay 0. */
    .global cred_test_fork
cred_test_fork:
    mov rbx, {data}
    mov dword ptr [rbx + 0x500], 50
    mov edi, 1
    lea rsi, [rbx + 0x500]
    mov eax, 116
    syscall
    test rax, rax
    jnz 90f
    mov edi, 7
    mov esi, 7
    mov edx, 7
    mov eax, 119
    syscall
    test rax, rax
    jnz 90f
    mov eax, 57
    syscall
    test rax, rax
    jz 5f
    js 90f
    mov r12, rax
    mov rdi, r12
    lea rsi, [rbx + 0x600]
    xor edx, edx
    xor r10d, r10d
    mov eax, 61
    syscall
    cmp rax, r12
    jne 91f
    mov eax, dword ptr [rbx + 0x600]
    test eax, eax
    jz 1f
    shr eax, 8
    movzx edi, al
    jmp 99f
1:
    mov eax, 102
    syscall
    test rax, rax
    jnz 92f
    mov rdi, {plain_thread}
    lea rsi, [rbx + 0x3000]
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    mov eax, 56
    syscall
    test rax, rax
    jz 3f
    js 93f
2:
    cmp qword ptr [rbx + 0x708], 1
    je 4f
    pause
    jmp 2b
4:
    cmp dword ptr [rbx + 0x700], 2000
    jne 94f
    mov eax, 102
    syscall
    test rax, rax
    jnz 95f
    mov eax, 107
    syscall
    test rax, rax
    jnz 95f
    xor edi, edi
    jmp 99f
3:
    mov edi, 2000
    mov esi, 2000
    mov edx, 2000
    mov eax, 117
    syscall
    mov eax, 102
    syscall
    mov dword ptr [rbx + 0x700], eax
    mov qword ptr [rbx + 0x708], 1
    xor edi, edi
    mov eax, 60
    syscall
    ud2
5:
    mov rdi, rbx
    lea rsi, [rbx + 4]
    lea rdx, [rbx + 8]
    mov eax, 120
    syscall
    test rax, rax
    jnz 80f
    cmp dword ptr [rbx], 7
    jne 80f
    cmp dword ptr [rbx + 8], 7
    jne 80f
    xor edi, edi
    lea rsi, [rbx + 0x680]
    mov eax, 115
    syscall
    cmp rax, 1
    jne 81f
    mov edi, 1
    lea rsi, [rbx + 0x680]
    mov eax, 115
    syscall
    cmp dword ptr [rbx + 0x680], 50
    jne 81f
    mov eax, 102
    syscall
    test rax, rax
    jnz 82f
    mov edi, 1000
    mov esi, 1000
    mov edx, 1000
    mov eax, 117
    syscall
    test rax, rax
    jnz 82f
    mov eax, 110
    syscall
    mov rdi, rax
    xor esi, esi
    mov eax, 62
    syscall
    cmp rax, -1
    jne 83f
    xor edi, edi
    jmp 99f
80: mov edi, 80
    jmp 99f
81: mov edi, 81
    jmp 99f
82: mov edi, 82
    jmp 99f
83: mov edi, 83
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
99: mov eax, 231
    syscall
    ud2

    .balign 16
    .global cred_test_end
cred_test_end:

/* The program the exec tests run, position independent: it exits with
   bit 0 set if its effective uid is 0, bit 1 if its real uid is 1000,
   bit 2 if its saved uid is 0, bit 3 if it has effective capabilities. */
    .balign 16
    .global cred_report_start
cred_report_start:
    sub rsp, 64
    mov rdi, rsp
    lea rsi, [rsp + 4]
    lea rdx, [rsp + 8]
    mov eax, 118
    syscall
    xor r12d, r12d
    cmp dword ptr [rsp + 4], 0
    jne 1f
    or r12d, 1
1:
    cmp dword ptr [rsp], 1000
    jne 2f
    or r12d, 2
2:
    cmp dword ptr [rsp + 8], 0
    jne 3f
    or r12d, 4
3:
    mov dword ptr [rsp + 16], 0x20080522
    mov dword ptr [rsp + 20], 0
    lea rdi, [rsp + 16]
    lea rsi, [rsp + 24]
    mov eax, 125
    syscall
    mov eax, dword ptr [rsp + 24]
    or eax, dword ptr [rsp + 36]
    test eax, eax
    jz 4f
    or r12d, 8
4:
    mov edi, r12d
    mov eax, 231
    syscall
    ud2
    .global cred_report_end
cred_report_end:
"#,
    data = const DATA,
    a = const A,
    b = const B,
    c = const C,
    exec = const EXEC,
    test_open = const TEST_OPEN,
    test_unlink = const TEST_UNLINK,
    plain_thread = const PLAIN_THREAD,
);

unsafe extern "C" {
    static cred_test_start: u8;
    static cred_test_end: u8;
    static cred_test_drop: u8;
    static cred_test_suid: u8;
    static cred_test_exec_root: u8;
    static cred_test_keepcaps: u8;
    static cred_test_capdrop: u8;
    static cred_test_sticky: u8;
    static cred_test_chown: u8;
    static cred_test_fork: u8;
    static cred_report_start: u8;
    static cred_report_end: u8;
}

/// The bytes between two symbols of the assembly above.
fn bytes(start: *const u8, end: *const u8) -> &'static [u8] {
    let len = end as usize - start as usize;
    // SAFETY: the assembly puts both symbols in one read-only section of
    // the kernel image, `start` first; the bytes between them are code
    // that lives as long as the kernel.
    unsafe { core::slice::from_raw_parts(start, len) }
}

fn blob() -> &'static [u8] {
    bytes(&raw const cred_test_start, &raw const cred_test_end)
}

/// A program's address once the blob is mapped at [`CODE`].
fn at(label: *const u8) -> u64 {
    CODE + (label as u64 - (&raw const cred_test_start) as u64)
}

/// A static x86-64 ELF executable (System V gABI, `<elf.h>` constants)
/// with one read-execute `PT_LOAD` segment holding the headers and the
/// report program.
fn report_elf() -> Vec<u8> {
    const BASE: u64 = 0x40_0000;
    const HEADERS: u64 = 64 + 56;
    let code = bytes(&raw const cred_report_start, &raw const cred_report_end);
    let size = HEADERS + code.len() as u64;
    let mut e = Vec::new();
    e.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    e.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    e.extend_from_slice(&62u16.to_le_bytes()); // EM_X86_64
    e.extend_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
    e.extend_from_slice(&(BASE + HEADERS).to_le_bytes()); // e_entry
    e.extend_from_slice(&64u64.to_le_bytes()); // e_phoff
    e.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
    e.extend_from_slice(&0u32.to_le_bytes()); // e_flags
    for half in [64u16, 56, 1, 64, 0, 0] {
        // e_ehsize, e_phentsize, e_phnum, e_shentsize, e_shnum, e_shstrndx
        e.extend_from_slice(&half.to_le_bytes());
    }
    e.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    e.extend_from_slice(&5u32.to_le_bytes()); // PF_R | PF_X
    for word in [0, BASE, BASE, size, size, 0x1000] {
        // p_offset, p_vaddr, p_paddr, p_filesz, p_memsz, p_align
        e.extend_from_slice(&word.to_le_bytes());
    }
    e.extend_from_slice(code);
    e
}

/// The path at `path` in the calling user thread's memory (at most 255
/// bytes, NUL-terminated).
fn user_path(addr: u64) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    user::copy_from_user(&mut buf, addr).ok()?;
    let n = buf.iter().position(|&b| b == 0)?;
    Some(buf[..n].to_vec())
}

fn errno(e: poly_vfs::Errno) -> u64 {
    (-(e as i64)) as u64
}

/// The default hook plus [`TEST_OPEN`] and [`TEST_UNLINK`], which use the
/// process's `Files` on the calling thread (so with its credentials).
fn hook(process: &Process, regs: &mut SyscallRegs) {
    let efault = errno(poly_vfs::Errno::EFAULT);
    match regs.rax {
        TEST_OPEN => {
            regs.rax = match user_path(regs.rdi) {
                Some(p) => match process.files.open(&p, OpenFlags::from_bits(regs.rsi as u32), regs.rdx as u16) {
                    Ok(fd) => fd as u64,
                    Err(e) => errno(e),
                },
                None => efault,
            };
        }
        TEST_UNLINK => {
            regs.rax = match user_path(regs.rdi) {
                Some(p) => process.files.unlinkat(fs::AT_FDCWD, &p, 0).map_or_else(errno, |()| 0),
                None => efault,
            };
        }
        _ => user::DEFAULT_HOOK(process, regs),
    }
}

/// A Linux process (root, a root of its own tree) and a memory with the
/// programs, the data pages (`prepare` fills them) and a stack.
fn setup(prepare: impl FnOnce(&Memory) -> TestResult) -> TestResult<(Arc<Process>, Arc<Memory>)> {
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
    prepare(&memory)?;
    Ok((process, Arc::new(memory)))
}

/// Write `path` (NUL-terminated) at `DATA + offset`.
fn put_path(memory: &Memory, offset: u64, path: &[u8]) -> TestResult {
    let mut p = path.to_vec();
    p.push(0);
    memory.write(DATA + offset, &p).map_err(|_| "writing a path failed")
}

/// `path` at [`A`], [`B`] or [`C`] and, for exec tests, the program at
/// [`EXEC`] with its argv.
fn paths(memory: &Memory, list: &[(u64, &[u8])]) -> TestResult {
    for &(offset, path) in list {
        put_path(memory, offset, path)?;
        if offset == EXEC {
            let mut argv = [0u8; 16];
            argv[..8].copy_from_slice(&(DATA + EXEC).to_le_bytes());
            memory.write(DATA + 0x300, &argv).map_err(|_| "writing argv failed")?;
        }
    }
    Ok(())
}

/// Wait until every user frame the test took is free again.
fn frames_back_to(before: u64) -> TestResult {
    let deadline = time::now_ns() + DEADLINE_NS;
    while user::user_frames() != before {
        if time::now_ns() > deadline {
            crate::kprintln!("linux cred test: {} user frames, {before} before", user::user_frames());
            return Err("user frames were not freed after the processes ended");
        }
        crate::thread::sleep(core::time::Duration::from_millis(2));
    }
    Ok(())
}

/// Run `program` as a fresh root process with [`hook`]; its exit status
/// once it and its memory are gone.
fn run(program: *const u8, prepare: impl FnOnce(&Memory) -> TestResult) -> TestResult<ExitStatus> {
    let before = user::user_frames();
    let status = {
        let (process, memory) = setup(prepare)?;
        let entry = Entry { rip: at(program), rsp: memory.layout().stack_top - 64, args: [0; 3] };
        let tid = user::spawn_in(Arc::clone(&process), memory, entry, hook).map_err(|_| "spawn failed")?;
        let _ = crate::thread::detach(tid);
        process.wait_until(time::now_ns() + DEADLINE_NS).ok_or("the test process did not end in time")?
    };
    frames_back_to(before)?;
    Ok(status)
}

fn expect_exit(status: ExitStatus, code: u8, what: &'static str) -> TestResult {
    if status == ExitStatus::Exited(code) {
        Ok(())
    } else {
        crate::kprintln!("linux cred test: {what}: {status}");
        Err(what)
    }
}

/// A file at `path` with `data`, owned by `uid:gid` with mode `perm`,
/// made by the kernel (as root).
fn make_file(files: &Files, path: &[u8], data: &[u8], uid: u32, gid: u32, perm: u16) -> TestResult {
    let _ = files.unlinkat(fs::AT_FDCWD, path, 0);
    let fd = files.open(path, OpenFlags::CREAT | OpenFlags::WRONLY | OpenFlags::TRUNC, 0o600).map_err(|e| e.name())?;
    let written = files.write(fd, data);
    let _ = files.close(fd);
    written.map_err(|e| e.name())?;
    files.chownat(fs::AT_FDCWD, path, Some(uid), Some(gid)).map_err(|e| e.name())?;
    files.chmodat(fs::AT_FDCWD, path, perm).map_err(|e| e.name())
}

/// Removes the test's files when it ends, however it ends.
struct Cleanup<'a>(&'a Files, &'a [&'a [u8]]);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for path in self.1 {
            let _ = self.0.unlinkat(fs::AT_FDCWD, path, 0);
        }
    }
}

const SECRET: &[u8] = b"/tmp/cred-secret";
const ALICES: &[u8] = b"/tmp/cred-alice";
const REPORT: &[u8] = b"/tmp/cred-report";
const SUID: &[u8] = b"/tmp/cred-suid";

kernel_test! {
    fn dropping_root_with_setresuid_loses_access_to_roots_files() -> TestResult {
        let files = Files::new();
        let _cleanup = Cleanup(&files, &[SECRET]);
        make_file(&files, SECRET, b"secret", 0, 0, 0o600)?;
        let status = run(&raw const cred_test_drop, |m| paths(m, &[(A, SECRET)]))?;
        expect_exit(status, 0, "dropping root: a check failed (see the program for the number)")
    }
}

kernel_test! {
    fn the_sticky_tmp_protects_other_users_files() -> TestResult {
        const MINE: &[u8] = b"/tmp/cred-mine";
        const THEIRS: &[u8] = b"/tmp/cred-theirs";
        let files = Files::new();
        let _cleanup = Cleanup(&files, &[MINE, THEIRS]);
        let tmp = files.stat(b"/tmp").map_err(|e| e.name())?;
        if tmp.perm() != 0o1777 {
            return Err("/tmp is not 01777");
        }
        make_file(&files, THEIRS, b"x", 1000, 1000, 0o666)?;
        let status = run(&raw const cred_test_sticky, |m| paths(m, &[(A, MINE), (C, THEIRS)]))?;
        expect_exit(status, 0, "sticky /tmp: a check failed")?;
        if files.stat(THEIRS).is_err() || files.stat(MINE).is_ok() {
            return Err("sticky /tmp: the wrong files were removed");
        }
        Ok(())
    }
}

kernel_test! {
    fn chown_and_chmod_follow_ownership_and_groups() -> TestResult {
        let files = Files::new();
        let _cleanup = Cleanup(&files, &[SECRET, ALICES]);
        make_file(&files, SECRET, b"secret", 0, 0, 0o644)?;
        make_file(&files, ALICES, b"x", 1000, 1000, 0o6755)?;
        let status = run(&raw const cred_test_chown, |m| paths(m, &[(A, SECRET), (B, ALICES)]))?;
        expect_exit(status, 0, "chown rules: a check failed")?;
        let st = files.stat(ALICES).map_err(|e| e.name())?;
        // chown to group 50 cleared both bits (group executable); chmod
        // set set-group-ID again (the caller is in group 50); the write
        // cleared it.
        if (st.uid, st.gid, st.perm()) != (1000, 50, 0o755) {
            crate::kprintln!("linux cred test: {:?} {} {} {:o}", core::str::from_utf8(ALICES), st.uid, st.gid, st.perm());
            return Err("chown rules: owner, group or mode not as expected");
        }
        let root = files.stat(SECRET).map_err(|e| e.name())?;
        if root.perm() != 0o644 {
            return Err("chown rules: root's file changed mode");
        }
        Ok(())
    }
}

kernel_test! {
    fn capabilities_dropped_with_prctl_and_capset_stay_dropped() -> TestResult {
        let files = Files::new();
        let _cleanup = Cleanup(&files, &[ALICES]);
        make_file(&files, ALICES, b"x", 1000, 1000, 0o600)?;
        let status = run(&raw const cred_test_capdrop, |m| paths(m, &[(B, ALICES)]))?;
        expect_exit(status, 0, "capability drop: a check failed")?;
        let st = files.stat(ALICES).map_err(|e| e.name())?;
        if (st.uid, st.perm()) != (1000, 0o644) {
            return Err("capability drop: chmod as root without CAP_DAC_OVERRIDE did not apply");
        }
        Ok(())
    }
}

/// Write the report program to `path` (root's) with mode `perm`.
fn make_report(files: &Files, path: &[u8], perm: u16) -> TestResult {
    make_file(files, path, &report_elf(), 0, 0, perm)
}

/// The no_new_privs switch of [`cred_test_suid`].
fn no_new_privs(memory: &Memory) -> TestResult {
    memory.write(DATA + 0x800, &1u64.to_le_bytes()).map_err(|_| "writing the switch failed")
}

kernel_test! {
    fn a_set_user_id_root_program_runs_with_root_privileges() -> TestResult {
        let files = Files::new();
        let _cleanup = Cleanup(&files, &[SUID]);
        make_report(&files, SUID, 0o4755)?;
        // euid 0, real 1000, saved 0, capabilities: 1 | 2 | 4 | 8.
        let status = run(&raw const cred_test_suid, |m| paths(m, &[(EXEC, SUID)]))?;
        expect_exit(status, 15, "a set-user-ID-root program did not get euid 0 and root's capabilities")?;
        // no_new_privs: the bit is ignored; real 1000 only.
        let status = run(&raw const cred_test_suid, |m| {
            paths(m, &[(EXEC, SUID)])?;
            no_new_privs(m)
        })?;
        expect_exit(status, 2, "with no_new_privs the set-user-ID bit was honoured")?;
        // Without the bit: nothing gained.
        files.chmodat(fs::AT_FDCWD, SUID, 0o755).map_err(|e| e.name())?;
        let status = run(&raw const cred_test_suid, |m| paths(m, &[(EXEC, SUID)]))?;
        expect_exit(status, 2, "a plain program changed the ids")
    }
}

kernel_test! {
    fn execve_recalculates_capabilities() -> TestResult {
        let files = Files::new();
        let _cleanup = Cleanup(&files, &[REPORT, SECRET]);
        make_report(&files, REPORT, 0o755)?;
        make_file(&files, SECRET, b"secret", 7, 7, 0o600)?;
        // Root with an empty effective set gets it back: euid 0, real 0,
        // saved 0, capabilities: 1 | 4 | 8.
        let status = run(&raw const cred_test_exec_root, |m| paths(m, &[(A, SECRET), (EXEC, REPORT)]))?;
        expect_exit(status, 13, "execve as root did not restore the effective capabilities")?;
        // keep_caps keeps the permitted set through setresuid (and
        // CAP_DAC_OVERRIDE works once effective), execve clears it: real
        // 1000 only.
        let status = run(&raw const cred_test_keepcaps, |m| paths(m, &[(A, SECRET), (EXEC, REPORT)]))?;
        expect_exit(status, 2, "keep_caps / execve: a check failed")
    }
}

kernel_test! {
    fn credentials_are_inherited_by_fork_and_kept_per_thread() -> TestResult {
        let status = run(&raw const cred_test_fork, |_| Ok(()))?;
        expect_exit(status, 0, "fork / thread credentials: a check failed (80+: in the child)")
    }
}

kernel_test! {
    fn kernel_threads_are_root_and_keep_set_id_bits_on_write() -> TestResult {
        // The kernel's own threads are root (CAP_FSETID): a write keeps
        // the bits (a user's write clears them: the chown test).
        let files = Files::new();
        let _cleanup = Cleanup(&files, &[ALICES]);
        make_file(&files, ALICES, b"x", 1000, 1000, 0o6777)?;
        let fd = files.open(ALICES, OpenFlags::WRONLY, 0).map_err(|e| e.name())?;
        let written = files.write(fd, b"more");
        let _ = files.close(fd);
        written.map_err(|e| e.name())?;
        if files.stat(ALICES).map_err(|e| e.name())?.perm() != 0o6777 {
            return Err("a write with CAP_FSETID cleared set-user-ID");
        }
        if files.cred().uid != 0 || !files.cred().is_root() {
            return Err("a kernel thread is not root");
        }
        let alice = poly_vfs::Cred::user(1000, 1000);
        files.access_as(&alice, ALICES, poly_vfs::Access::WRITE).map_err(|e| e.name())
    }
}
