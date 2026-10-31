//! The loader end to end (ADR 0071): three self-built PE images whose
//! imports, relocations, TLS, `DllMain`s and delay-load import are all
//! resolved by our real `ntdll` (`poly-ldr` inside it) in ring 3. No
//! Microsoft toolchain or DLLs: the code is assembled here and the images
//! are written with `poly_ldr::builder`.
//!
//! - `ldrtest.exe` imports heap, string, thread and process functions from
//!   `ntdll.dll` and three functions from `polyldr.dll`, has a TLS
//!   directory with one callback, and delay-loads `PolyDelayed` from
//!   `polydly.dll` through a thunk that calls `LdrResolveDelayLoadedAPI`.
//! - `polyldr.dll` prefers the executable's base, so the loader must map it
//!   elsewhere and apply its relocations: its code reaches its data, its
//!   TLS index and its import of `NtTerminateProcess` only through
//!   absolute addresses in a link table that `.reloc` covers. It has two
//!   TLS callbacks, a TLS template and a `DllMain`.
//! - `polydly.dll` imports `PolyLdrLog` from `polyldr.dll` and has a
//!   `DllMain`.
//!
//! Every TLS callback and `DllMain` appends `(who << 4) | reason` to a log
//! in `polyldr.dll`'s data (`who`: 1 and 2 its TLS callbacks, 3 its
//! `DllMain`, 4 the executable's TLS callback, 5 `polydly.dll`'s
//! `DllMain`). The expected log ([`EXPECTED`]) is ADR 0071's order:
//! dependencies first and a module's TLS callbacks before its entry point
//! on attach; reverse initialisation order on detach.
//!
//! The executable checks, reporting a failed step with `PolyTrace(0xBADBAD,
//! step, rax)` and exit code `0xE0 + step`: the attach order; that the
//! DLL's and its own TLS blocks hold their templates; that the DLL's data
//! is not at its preferred-base address (it was relocated); a private heap
//! (`RtlCreateHeap`, a zeroed and 16-byte aligned `RtlAllocateHeap`,
//! `RtlSizeHeap`, `RtlFreeHeap`, `RtlDestroyHeap`); `RtlInitUnicodeString`;
//! the delay-load (the first call loads `polydly.dll` and runs its
//! `DllMain`, the slot is then bound, a second call does not load again);
//! a thread (`NtCreateThreadEx`), which gets fresh TLS blocks and
//! `DLL_THREAD_ATTACH`/`DETACH` in every module, while the main thread's
//! own TLS value stays. It ends with `RtlExitUserProcess(PASS_CODE)`;
//! `polyldr.dll`'s `DllMain(DLL_PROCESS_DETACH)`, the last callout, checks
//! the whole log and ends the process with `0xDD` if it is wrong.
//!
//! Every `global_asm!` label is prefixed `ldrexe_`, `ldrdll_` or `ldrdly_`.

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_ldr::builder::{DATA, Export, Import, PeBuilder, Section, Sym, TEXT, Target, Tls};

/// The exit status when every check passed.
pub const PASS_CODE: u8 = 0x5A;
/// The exit status when `polyldr.dll`'s final check of the log failed.
pub const DETACH_FAIL_CODE: u8 = 0xDD;
/// The executable's preferred base, which `polyldr.dll` prefers too.
pub const IMAGE_BASE: u64 = 0x1_4000_0000;
/// `polydly.dll`'s preferred base.
pub const DELAY_BASE: u64 = 0x1_8000_0000;
/// Where the executable "is" (its process parameters) and its DLLs are.
pub const APP_DIR: &str = "Program Files\\PolyLdrTest";
pub const APP_EXE: &str = "C:\\Program Files\\PolyLdrTest\\ldrtest.exe";

/// The log every callout appends to, in order (see the module docs).
pub const EXPECTED: [u8; 20] = [
    0x11, 0x21, 0x31, 0x41, // process start: polyldr (TLS, TLS, DllMain), then the executable's TLS
    0x51, // the delay-load brings polydly in
    0x12, 0x22, 0x32, 0x42, 0x52, // a thread starts: initialisation order
    0x53, 0x43, 0x13, 0x23, 0x33, // and ends: reverse order
    0x50, 0x40, 0x10, 0x20, 0x30, // process exit: reverse order
];
/// How many entries [`EXPECTED`] has.
pub const EXPECTED_LEN: u64 = EXPECTED.len() as u64;

/// The DLL's TLS template's first value and the executable's.
const DLL_TLS: u64 = 0x1122_3344_5566_7788;
const EXE_TLS: u64 = 0xE7E7_E7E7_E7E7_E7E7;

// The executable's link table: the addresses its code needs, one slot each
// (byte offsets into `ldrexe_links`), written when the image is built.
const L_LOG: usize = 0x00; // IAT: polyldr!PolyLdrLog
const L_LOG_PTR: usize = 0x08; // IAT: polyldr!PolyLdrLogPtr
const L_TLS_VALUE: usize = 0x10; // IAT: polyldr!PolyLdrTlsValue
const L_CREATE_HEAP: usize = 0x18; // IAT: ntdll!RtlCreateHeap
const L_ALLOC: usize = 0x20; // IAT: ntdll!RtlAllocateHeap
const L_SIZE: usize = 0x28; // IAT: ntdll!RtlSizeHeap
const L_FREE: usize = 0x30; // IAT: ntdll!RtlFreeHeap
const L_DESTROY: usize = 0x38; // IAT: ntdll!RtlDestroyHeap
const L_INIT_US: usize = 0x40; // IAT: ntdll!RtlInitUnicodeString
const L_EXIT: usize = 0x48; // IAT: ntdll!RtlExitUserProcess
const L_CREATE_THREAD: usize = 0x50; // IAT: ntdll!NtCreateThreadEx
const L_WAIT: usize = 0x58; // IAT: ntdll!NtWaitForSingleObject
const L_CLOSE: usize = 0x60; // IAT: ntdll!NtClose
const L_RESOLVE: usize = 0x68; // IAT: ntdll!LdrResolveDelayLoadedAPI
const L_DELAY_SLOT: usize = 0x70; // the delay-load address table slot
const L_DELAY_DESC: usize = 0x78; // the delay-load descriptor
const L_BASE: usize = 0x80; // the executable's base
const L_TLS_INDEX: usize = 0x88; // the executable's TLS index slot
const L_PREFERRED_LOG: usize = 0x90; // polyldr's log at its preferred base (not relocated)
const L_COUNT: usize = 19;

/// What the executable imports from `ntdll.dll`, in link-table order.
const NTDLL_IMPORTS: [(&str, usize); 11] = [
    ("RtlCreateHeap", L_CREATE_HEAP),
    ("RtlAllocateHeap", L_ALLOC),
    ("RtlSizeHeap", L_SIZE),
    ("RtlFreeHeap", L_FREE),
    ("RtlDestroyHeap", L_DESTROY),
    ("RtlInitUnicodeString", L_INIT_US),
    ("RtlExitUserProcess", L_EXIT),
    ("NtCreateThreadEx", L_CREATE_THREAD),
    ("NtWaitForSingleObject", L_WAIT),
    ("NtClose", L_CLOSE),
    ("LdrResolveDelayLoadedAPI", L_RESOLVE),
];
/// What it imports from `polyldr.dll`.
const DLL_IMPORTS: [(&str, usize); 3] =
    [("PolyLdrLog", L_LOG), ("PolyLdrLogPtr", L_LOG_PTR), ("PolyLdrTlsValue", L_TLS_VALUE)];

// polyldr.dll's link table.
const D_LOG: usize = 0x00; // its log (in .data)
const D_TLS_INDEX: usize = 0x08; // its TLS index slot
const D_TERMINATE: usize = 0x10; // IAT: ntdll!NtTerminateProcess
const D_COUNT: usize = 3;

global_asm!(
    r#"
    .section .rodata.poly_nt_ldr, "a", @progbits

    // ===== ldrtest.exe ====================================================
    .balign 16
    .global ldrexe_start
ldrexe_start:
    .global ldrexe_main
ldrexe_main:
    sub rsp, 0x208                // entered as if called: now 16-aligned
    lea rbx, [rsp+0x100]          // locals

    // 1. Process start ran the attach callouts in order.
    mov r15d, 1
    mov ecx, 4
    call ldrexe_check_log
    test eax, eax
    jnz ldrexe_fail

    // 2. TLS: the DLL's block and ours hold their templates.
    mov r15d, 2
    mov rax, [rip+ldrexe_links+{l_tls_value}]
    call [rax]
    mov rcx, {dll_tls}
    cmp rax, rcx
    jne ldrexe_fail
    mov r15d, 3
    call ldrexe_own_tls
    mov rax, [rax]
    mov rcx, {exe_tls}
    cmp rax, rcx
    jne ldrexe_fail

    // 3. polyldr.dll was relocated: its data is not where its preferred
    //    base would put it, yet its code found it.
    mov r15d, 4
    mov rax, [rip+ldrexe_links+{l_log_ptr}]
    call [rax]
    cmp rax, [rip+ldrexe_links+{l_preferred_log}]
    je ldrexe_fail

    // 4. A private heap.
    mov r15d, 5
    mov ecx, 2                    // HEAP_GROWABLE
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    mov qword ptr [rsp+0x20], 0
    mov qword ptr [rsp+0x28], 0
    mov rax, [rip+ldrexe_links+{l_create_heap}]
    call [rax]
    test rax, rax
    jz ldrexe_fail
    mov r12, rax
    mov r15d, 6
    mov rcx, r12
    mov edx, 8                    // HEAP_ZERO_MEMORY
    mov r8d, 100
    mov rax, [rip+ldrexe_links+{l_alloc}]
    call [rax]
    test rax, rax
    jz ldrexe_fail
    test al, 0xF
    jnz ldrexe_fail
    mov r13, rax
    cmp qword ptr [r13], 0
    jne ldrexe_fail
    cmp dword ptr [r13+96], 0
    jne ldrexe_fail
    mov qword ptr [r13], -1
    mov dword ptr [r13+96], -1
    mov r15d, 7
    mov rcx, r12
    xor edx, edx
    mov r8, r13
    mov rax, [rip+ldrexe_links+{l_size}]
    call [rax]
    cmp rax, 100
    jne ldrexe_fail
    mov r15d, 8
    mov rcx, r12
    xor edx, edx
    mov r8, r13
    mov rax, [rip+ldrexe_links+{l_free}]
    call [rax]
    test al, al
    jz ldrexe_fail
    mov r15d, 9
    mov rcx, r12
    mov rax, [rip+ldrexe_links+{l_destroy}]
    call [rax]
    test rax, rax
    jnz ldrexe_fail

    // 5. RtlInitUnicodeString(&s, L"Poly"): Length 8, MaximumLength 10.
    mov r15d, 10
    lea rcx, [rbx+0x00]
    lea rdx, [rip+ldrexe_poly_w]
    mov rax, [rip+ldrexe_links+{l_init_us}]
    call [rax]
    cmp word ptr [rbx+0x00], 8
    jne ldrexe_fail
    cmp word ptr [rbx+0x02], 10
    jne ldrexe_fail
    lea rax, [rip+ldrexe_poly_w]
    cmp [rbx+0x08], rax
    jne ldrexe_fail

    // 6. The delay-load: the first call loads polydly.dll (its DllMain
    //    logs 0x51) and binds the slot; the second goes straight there.
    mov r15d, 11
    mov rax, [rip+ldrexe_links+{l_delay_slot}]
    call [rax]
    cmp eax, 0x5150
    jne ldrexe_fail
    mov r15d, 12
    mov ecx, 5
    call ldrexe_check_log
    test eax, eax
    jnz ldrexe_fail
    mov r15d, 13
    mov rax, [rip+ldrexe_links+{l_delay_slot}]
    mov rax, [rax]
    lea rcx, [rip+ldrexe_delay_stub]
    cmp rax, rcx
    je ldrexe_fail
    mov rax, [rip+ldrexe_links+{l_delay_slot}]
    call [rax]
    cmp eax, 0x5150
    jne ldrexe_fail
    mov ecx, 5
    call ldrexe_check_log
    test eax, eax
    jnz ldrexe_fail

    // 7. A thread: our TLS value changes first; the thread must see the
    //    templates in fresh blocks.
    mov r15d, 14
    call ldrexe_own_tls
    mov qword ptr [rax], 0xAAAA
    mov qword ptr [rbx+0x40], 0
    mov qword ptr [rbx+0x48], 0
    lea rcx, [rbx+0x20]           // &ThreadHandle
    mov edx, 0x1FFFFF             // THREAD_ALL_ACCESS
    xor r8d, r8d
    mov r9, -1                    // NtCurrentProcess()
    lea rax, [rip+ldrexe_thread]
    mov [rsp+0x20], rax           // StartRoutine
    lea rax, [rbx+0x40]
    mov [rsp+0x28], rax           // Argument
    mov qword ptr [rsp+0x30], 0
    mov qword ptr [rsp+0x38], 0
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 0
    mov qword ptr [rsp+0x50], 0
    mov rax, [rip+ldrexe_links+{l_create_thread}]
    call [rax]
    test eax, eax
    jnz ldrexe_fail
    mov r15d, 15
    mov rcx, [rbx+0x20]
    xor edx, edx
    xor r8d, r8d
    mov rax, [rip+ldrexe_links+{l_wait}]
    call [rax]
    test eax, eax
    jnz ldrexe_fail
    mov r15d, 16
    mov rcx, {dll_tls}
    cmp [rbx+0x40], rcx
    jne ldrexe_fail
    mov rcx, {exe_tls}
    cmp [rbx+0x48], rcx
    jne ldrexe_fail
    mov r15d, 17
    call ldrexe_own_tls
    cmp qword ptr [rax], 0xAAAA
    jne ldrexe_fail
    mov r15d, 18
    mov ecx, 15
    call ldrexe_check_log
    test eax, eax
    jnz ldrexe_fail
    mov r15d, 19
    mov rcx, [rbx+0x20]
    mov rax, [rip+ldrexe_links+{l_close}]
    call [rax]
    test eax, eax
    jnz ldrexe_fail

    // 8. Exit through ntdll: DLL_PROCESS_DETACH everywhere, then the end.
    mov ecx, {pass}
    mov rax, [rip+ldrexe_links+{l_exit}]
    call [rax]
    ud2

ldrexe_fail:
    // PolyTrace(0xBADBAD, step, rax), then NtTerminateProcess(-1, 0xE0 +
    // step), straight to the kernel.
    mov r8, rax
    mov edx, r15d
    mov r10, 0xBADBAD
    mov eax, 0x3000
    syscall
    mov r10, -1
    lea edx, [r15+0xE0]
    mov eax, 0x80
    syscall
    ud2

    // Compare the log with the first ECX expected entries: EAX 0 if equal.
    .balign 16
ldrexe_check_log:
    push rbx
    sub rsp, 0x20
    mov ebx, ecx
    mov rax, [rip+ldrexe_links+{l_log_ptr}]
    call [rax]
    cmp [rax], rbx
    jne ldrexe_check_bad
    lea rdx, [rip+ldrexe_expected]
    xor ecx, ecx
ldrexe_check_next:
    cmp ecx, ebx
    jae ldrexe_check_ok
    mov r8b, [rax+rcx+8]
    cmp r8b, [rdx+rcx]
    jne ldrexe_check_bad
    inc ecx
    jmp ldrexe_check_next
ldrexe_check_ok:
    xor eax, eax
    jmp ldrexe_check_out
ldrexe_check_bad:
    mov eax, 1
ldrexe_check_out:
    add rsp, 0x20
    pop rbx
    ret

    // RAX = this thread's block of our own TLS (gs:[0x58] is the TEB's
    // ThreadLocalStoragePointer, indexed by our _tls_index).
ldrexe_own_tls:
    mov rax, [rip+ldrexe_links+{l_tls_index}]
    mov eax, [rax]
    mov rdx, gs:[0x58]
    mov rax, [rdx+rax*8]
    ret

    // The thread: store the DLL's and our TLS values where the argument
    // points, exit with 0x77.
    .balign 16
    .global ldrexe_thread
ldrexe_thread:
    push rbx
    sub rsp, 0x20
    mov rbx, rcx
    mov rax, [rip+ldrexe_links+{l_tls_value}]
    call [rax]
    mov [rbx+0x00], rax
    call ldrexe_own_tls
    mov rax, [rax]
    mov [rbx+0x08], rax
    mov eax, 0x77
    add rsp, 0x20
    pop rbx
    ret

    // Our TLS callback: log (4 << 4) | reason.
    .balign 16
    .global ldrexe_tls_cb
ldrexe_tls_cb:
    lea ecx, [rdx+0x40]
    mov rax, [rip+ldrexe_links+{l_log}]
    jmp [rax]

    // The delay-load thunk the slot starts out at: resolve through
    // LdrResolveDelayLoadedAPI(base, descriptor, NULL, NULL, &slot, 0) and
    // jump to the result with the caller's arguments.
    .balign 16
    .global ldrexe_delay_stub
ldrexe_delay_stub:
    push rcx
    push rdx
    push r8
    push r9
    sub rsp, 0x38
    mov rcx, [rip+ldrexe_links+{l_base}]
    mov rdx, [rip+ldrexe_links+{l_delay_desc}]
    xor r8d, r8d
    xor r9d, r9d
    mov rax, [rip+ldrexe_links+{l_delay_slot}]
    mov [rsp+0x20], rax
    mov qword ptr [rsp+0x28], 0
    mov rax, [rip+ldrexe_links+{l_resolve}]
    call [rax]
    add rsp, 0x38
    pop r9
    pop r8
    pop rdx
    pop rcx
    jmp rax

    .balign 2
ldrexe_poly_w:
    .2byte 'P', 'o', 'l', 'y', 0
    .balign 8
ldrexe_expected:
    .byte {e0}, {e1}, {e2}, {e3}, {e4}, {e5}, {e6}, {e7}, {e8}, {e9}
    .byte {e10}, {e11}, {e12}, {e13}, {e14}, {e15}, {e16}, {e17}, {e18}, {e19}
    .balign 8
    .global ldrexe_links
ldrexe_links:
    .fill {l_count}, 8, 0
    .global ldrexe_end
ldrexe_end:

    // ===== polyldr.dll ====================================================
    .balign 16
    .global ldrdll_start
ldrdll_start:
    // void PolyLdrLog(BYTE code): append to the log (at most 0xF0).
    .global ldrdll_log
ldrdll_log:
    mov rax, [rip+ldrdll_links+{d_log}]
    mov rdx, [rax]
    cmp rdx, 0xF0
    jae ldrdll_log_full
    mov [rax+rdx+8], cl
    inc rdx
    mov [rax], rdx
ldrdll_log_full:
    ret

    // BYTE *PolyLdrLogPtr(void): the log (count, then the entries).
    .balign 16
    .global ldrdll_log_ptr
ldrdll_log_ptr:
    mov rax, [rip+ldrdll_links+{d_log}]
    ret

    // ULONG64 PolyLdrTlsValue(void): the first value of this thread's
    // block of our TLS.
    .balign 16
    .global ldrdll_tls_value
ldrdll_tls_value:
    mov rax, [rip+ldrdll_links+{d_tls_index}]
    mov eax, [rax]
    mov rdx, gs:[0x58]
    mov rdx, [rdx+rax*8]
    mov rax, [rdx]
    ret

    // The TLS callbacks (DllHandle, Reason, Reserved): log 0x10 | reason
    // and 0x20 | reason.
    .balign 16
    .global ldrdll_tls_cb1
ldrdll_tls_cb1:
    lea ecx, [rdx+0x10]
    jmp ldrdll_log
    .balign 16
    .global ldrdll_tls_cb2
ldrdll_tls_cb2:
    lea ecx, [rdx+0x20]
    jmp ldrdll_log

    // DllMain(hinstDLL, fdwReason, lpvReserved): log 0x30 | reason; at
    // DLL_PROCESS_DETACH check the whole log.
    .balign 16
    .global ldrdll_main
ldrdll_main:
    push rbx
    sub rsp, 0x20
    mov ebx, edx
    lea ecx, [rdx+0x30]
    call ldrdll_log
    test ebx, ebx
    jnz ldrdll_main_out
    mov rax, [rip+ldrdll_links+{d_log}]
    cmp qword ptr [rax], {expected_len}
    jne ldrdll_bad
    lea rdx, [rip+ldrdll_expected]
    xor ecx, ecx
ldrdll_check_next:
    cmp ecx, {expected_len}
    jae ldrdll_main_out
    mov r8b, [rax+rcx+8]
    cmp r8b, [rdx+rcx]
    jne ldrdll_bad
    inc ecx
    jmp ldrdll_check_next
ldrdll_main_out:
    mov eax, 1
    add rsp, 0x20
    pop rbx
    ret
ldrdll_bad:
    // NtTerminateProcess(NtCurrentProcess(), 0xDD) through our import.
    mov rcx, -1
    mov edx, {detach_fail}
    mov rax, [rip+ldrdll_links+{d_terminate}]
    call [rax]
    ud2

ldrdll_expected:
    .byte {e0}, {e1}, {e2}, {e3}, {e4}, {e5}, {e6}, {e7}, {e8}, {e9}
    .byte {e10}, {e11}, {e12}, {e13}, {e14}, {e15}, {e16}, {e17}, {e18}, {e19}
    .balign 8
    .global ldrdll_links
ldrdll_links:
    .fill {d_count}, 8, 0
    .global ldrdll_end
ldrdll_end:

    // ===== polydly.dll ====================================================
    .balign 16
    .global ldrdly_start
ldrdly_start:
    // DllMain: log 0x50 | reason through polyldr!PolyLdrLog.
    .global ldrdly_main
ldrdly_main:
    sub rsp, 0x28
    lea ecx, [rdx+0x50]
    mov rax, [rip+ldrdly_link_log]
    call [rax]
    mov eax, 1
    add rsp, 0x28
    ret

    // ULONG PolyDelayed(void).
    .balign 16
    .global ldrdly_delayed
ldrdly_delayed:
    mov eax, 0x5150
    ret

    .balign 8
    .global ldrdly_link_log
ldrdly_link_log:
    .quad 0
    .global ldrdly_end
ldrdly_end:
"#,
    pass = const PASS_CODE as u32,
    detach_fail = const DETACH_FAIL_CODE as u32,
    dll_tls = const DLL_TLS,
    exe_tls = const EXE_TLS,
    expected_len = const EXPECTED_LEN,
    l_log = const L_LOG,
    l_log_ptr = const L_LOG_PTR,
    l_tls_value = const L_TLS_VALUE,
    l_create_heap = const L_CREATE_HEAP,
    l_alloc = const L_ALLOC,
    l_size = const L_SIZE,
    l_free = const L_FREE,
    l_destroy = const L_DESTROY,
    l_init_us = const L_INIT_US,
    l_exit = const L_EXIT,
    l_create_thread = const L_CREATE_THREAD,
    l_wait = const L_WAIT,
    l_close = const L_CLOSE,
    l_resolve = const L_RESOLVE,
    l_delay_slot = const L_DELAY_SLOT,
    l_delay_desc = const L_DELAY_DESC,
    l_base = const L_BASE,
    l_tls_index = const L_TLS_INDEX,
    l_preferred_log = const L_PREFERRED_LOG,
    l_count = const L_COUNT,
    d_log = const D_LOG,
    d_tls_index = const D_TLS_INDEX,
    d_terminate = const D_TERMINATE,
    d_count = const D_COUNT,
    e0 = const EXPECTED[0],
    e1 = const EXPECTED[1],
    e2 = const EXPECTED[2],
    e3 = const EXPECTED[3],
    e4 = const EXPECTED[4],
    e5 = const EXPECTED[5],
    e6 = const EXPECTED[6],
    e7 = const EXPECTED[7],
    e8 = const EXPECTED[8],
    e9 = const EXPECTED[9],
    e10 = const EXPECTED[10],
    e11 = const EXPECTED[11],
    e12 = const EXPECTED[12],
    e13 = const EXPECTED[13],
    e14 = const EXPECTED[14],
    e15 = const EXPECTED[15],
    e16 = const EXPECTED[16],
    e17 = const EXPECTED[17],
    e18 = const EXPECTED[18],
    e19 = const EXPECTED[19],
);

unsafe extern "C" {
    static ldrexe_start: u8;
    static ldrexe_main: u8;
    static ldrexe_tls_cb: u8;
    static ldrexe_delay_stub: u8;
    static ldrexe_links: u8;
    static ldrexe_end: u8;
    static ldrdll_start: u8;
    static ldrdll_log: u8;
    static ldrdll_log_ptr: u8;
    static ldrdll_tls_value: u8;
    static ldrdll_tls_cb1: u8;
    static ldrdll_tls_cb2: u8;
    static ldrdll_main: u8;
    static ldrdll_links: u8;
    static ldrdll_end: u8;
    static ldrdly_start: u8;
    static ldrdly_main: u8;
    static ldrdly_delayed: u8;
    static ldrdly_link_log: u8;
    static ldrdly_end: u8;
}

/// One image's assembled code: the bytes between two labels.
struct Code {
    start: *const u8,
    bytes: Vec<u8>,
}

impl Code {
    fn new(start: *const u8, end: *const u8) -> Code {
        // SAFETY: the two labels bound one run of bytes in a read-only
        // section of the kernel image.
        let bytes = unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }.to_vec();
        Code { start, bytes }
    }

    /// The offset of `label` in the code (the code goes at RVA 0x1000, so
    /// its RVA is this plus 0x1000).
    fn at(&self, label: *const u8) -> usize {
        label as usize - self.start as usize
    }

    fn rva(&self, label: *const u8) -> u32 {
        0x1000 + self.at(label) as u32
    }
}

/// Write `value` into link slot `slot` of the table at `table` in section 0
/// of `b`, relocated with the image unless `fixed`.
fn link(b: &mut PeBuilder, table: usize, slot: usize, value: u64, fixed: bool) {
    let at = table + slot;
    b.sections[0].data[at..at + 8].copy_from_slice(&value.to_le_bytes());
    if !fixed {
        b.relocations.push(0x1000 + at as u32);
    }
}

fn export(name: &str, ordinal: u32, rva: u32) -> Export {
    Export { name: Some(name.into()), ordinal, target: Target::Rva(rva) }
}

/// `polyldr.dll`, and the address of its log at its preferred base.
pub fn dll_image() -> (Vec<u8>, u64) {
    let code = Code::new(&raw const ldrdll_start, &raw const ldrdll_end);
    let mut b = PeBuilder {
        image_base: IMAGE_BASE,
        dll: true,
        entry: code.rva(&raw const ldrdll_main),
        sections: vec![Section::new(".text", code.bytes.clone(), TEXT), Section::new(".data", vec![0; 0x100], DATA)],
        export_name: "polyldr.dll".into(),
        exports: vec![
            export("PolyLdrLog", 1, code.rva(&raw const ldrdll_log)),
            export("PolyLdrLogPtr", 2, code.rva(&raw const ldrdll_log_ptr)),
            export("PolyLdrTlsValue", 3, code.rva(&raw const ldrdll_tls_value)),
        ],
        imports: vec![Import {
            dll: "ntdll.dll".into(),
            symbols: vec![Sym::name("NtTerminateProcess")],
            ..Import::default()
        }],
        tls: Some(Tls {
            template: [DLL_TLS.to_le_bytes(), [0; 8]].concat(),
            zero_fill: 16,
            callbacks: vec![code.rva(&raw const ldrdll_tls_cb1), code.rva(&raw const ldrdll_tls_cb2)],
            align_flag: 0,
        }),
        ..PeBuilder::default()
    };
    let lay = b.layout();
    let log = IMAGE_BASE + u64::from(lay.sections[1]);
    let table = code.at(&raw const ldrdll_links);
    link(&mut b, table, D_LOG, log, false);
    link(&mut b, table, D_TLS_INDEX, IMAGE_BASE + u64::from(lay.tls_index.unwrap_or(0)), false);
    link(&mut b, table, D_TERMINATE, IMAGE_BASE + u64::from(lay.iat[0][0]), false);
    (b.write(), log)
}

/// `polydly.dll`.
pub fn delay_dll_image() -> Vec<u8> {
    let code = Code::new(&raw const ldrdly_start, &raw const ldrdly_end);
    let mut b = PeBuilder {
        image_base: DELAY_BASE,
        dll: true,
        entry: code.rva(&raw const ldrdly_main),
        sections: vec![Section::new(".text", code.bytes.clone(), TEXT)],
        export_name: "polydly.dll".into(),
        exports: vec![export("PolyDelayed", 1, code.rva(&raw const ldrdly_delayed))],
        imports: vec![Import {
            dll: "polyldr.dll".into(),
            symbols: vec![Sym::name("PolyLdrLog")],
            ..Import::default()
        }],
        ..PeBuilder::default()
    };
    let lay = b.layout();
    link(&mut b, code.at(&raw const ldrdly_link_log), 0, DELAY_BASE + u64::from(lay.iat[0][0]), false);
    b.write()
}

/// `ldrtest.exe`; `preferred_log` is `polyldr.dll`'s log at its preferred
/// base.
pub fn exe_image(preferred_log: u64) -> Vec<u8> {
    let code = Code::new(&raw const ldrexe_start, &raw const ldrexe_end);
    let symbols = |list: &[(&str, usize)]| list.iter().map(|(n, _)| Sym::name(n)).collect();
    let mut b = PeBuilder {
        image_base: IMAGE_BASE,
        dll: false,
        entry: code.rva(&raw const ldrexe_main),
        sections: vec![Section::new(".text", code.bytes.clone(), TEXT)],
        imports: vec![
            Import { dll: "ntdll.dll".into(), symbols: symbols(&NTDLL_IMPORTS), ..Import::default() },
            Import { dll: "polyldr.dll".into(), symbols: symbols(&DLL_IMPORTS), ..Import::default() },
        ],
        delay_imports: vec![Import {
            dll: "polydly.dll".into(),
            symbols: vec![Sym::name("PolyDelayed")],
            delay_thunks: vec![code.rva(&raw const ldrexe_delay_stub)],
            ..Import::default()
        }],
        tls: Some(Tls {
            template: EXE_TLS.to_le_bytes().to_vec(),
            zero_fill: 8,
            callbacks: vec![code.rva(&raw const ldrexe_tls_cb)],
            align_flag: 0,
        }),
        stack_reserve: 0x4_0000,
        stack_commit: 0x4000,
        ..PeBuilder::default()
    };
    let lay = b.layout();
    let table = code.at(&raw const ldrexe_links);
    let va = |rva: u32| IMAGE_BASE + u64::from(rva);
    for (i, &(_, slot)) in NTDLL_IMPORTS.iter().enumerate() {
        link(&mut b, table, slot, va(lay.iat[0][i]), false);
    }
    for (i, &(_, slot)) in DLL_IMPORTS.iter().enumerate() {
        link(&mut b, table, slot, va(lay.iat[1][i]), false);
    }
    link(&mut b, table, L_DELAY_SLOT, va(lay.delay_iat[0][0]), false);
    link(&mut b, table, L_DELAY_DESC, va(lay.delay_descriptors[0]), false);
    link(&mut b, table, L_BASE, IMAGE_BASE, false);
    link(&mut b, table, L_TLS_INDEX, va(lay.tls_index.unwrap_or(0)), false);
    link(&mut b, table, L_PREFERRED_LOG, preferred_log, true);
    b.write()
}

mod tests {
    use super::*;
    use crate::kernel_test;
    use crate::process::Process;
    use crate::thread;
    use crate::user::memory::Layout;
    use crate::user::{self, Memory};
    use alloc::string::String;
    use alloc::sync::Arc;
    use poly_binfmt::{Arch, Bits, Format};
    use poly_personality_nt::io::VfsRoot;
    use poly_personality_nt::layout::create;
    use poly_vfs::{At, Cred, OpenFlags};

    const FORMAT: Format = Format::Pe { bits: Bits::B64, arch: Arch::X86_64 };

    /// The whole file at VFS path `path`.
    fn read_file(path: &str) -> Option<Vec<u8>> {
        let vfs = crate::fs::vfs();
        let ctx = vfs.root_context().ok()?;
        let f = vfs.open(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), OpenFlags::RDONLY, 0).ok()?;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match f.read_at(out.len() as u64, &mut buf).ok()? {
                0 => return Some(out),
                n => out.extend_from_slice(&buf[..n]),
            }
        }
    }

    kernel_test! {
        /// Our `ntdll` loads a program end to end (ADR 0071): the kernel
        /// maps `ldrtest.exe` and `ntdll`, and `ntdll`'s loader binds the
        /// executable's imports from `ntdll` and from `polyldr.dll` (found
        /// in the application's directory and relocated, since it prefers
        /// the executable's base), runs TLS callbacks and `DllMain`s in
        /// order, delay-loads `polydly.dll`, starts a thread with its own
        /// TLS and thread callouts, and runs every `DLL_PROCESS_DETACH` at
        /// `RtlExitUserProcess`. See [`super`].
        fn nt_ldr_binds_imports_tls_dllmain_and_delay_loads_through_ntdll() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the loader test");
            }
            if super::super::NTDLL.is_empty() {
                return Err("this kernel was built without ntdll.dll");
            }
            // The first NT use makes C: (ADR 0073) and puts ntdll.dll in
            // System32.
            super::super::with(|_| ());
            let in_system32 = read_file("/nt/c/Windows/System32/ntdll.dll");
            if in_system32.as_deref() != Some(super::super::NTDLL) {
                return Err("System32 does not hold our ntdll.dll");
            }
            // The DLLs go in the application's directory.
            let (dll, preferred_log) = dll_image();
            let vfs = crate::fs::vfs();
            let ctx = vfs.root_context().map_err(|_| "no VFS")?;
            let root = vfs.lookup(&ctx, &Cred::root(), At::Cwd, b"/nt/c", true).map_err(|_| "no C:")?;
            let vr = VfsRoot::new(vfs, ctx, Cred::root());
            for (name, bytes) in [("polyldr.dll", dll), ("polydly.dll", delay_dll_image())] {
                create::install_file(&vr, &root, APP_DIR, name, &bytes).map_err(|_| "cannot write a test DLL")?;
            }

            let file = exe_image(preferred_log);
            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch { image_path: String::from(APP_EXE), args: Vec::new() };
            let start = super::super::start_with(&process, &memory, &file, pid, &launch)
                .map_err(|_| "the kernel rejected the test executable")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!(
                    "nt: the loader test ended with {code:#x}, not {PASS_CODE:#x} (0xE0 + step: a failed step; {DETACH_FAIL_CODE:#x}: the detach log)"
                );
                return Err("the loader test program did not pass every check");
            }
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the NT process kept its memory after it exited");
            }
            if super::super::is_nt_process(pid) {
                return Err("the NT process state was not cleaned up");
            }
            Ok(())
        }
    }
}
