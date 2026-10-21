//! A self-assembled ring-3 PE that drives the NT registry services end to
//! end (ADR 0066). No Microsoft toolchain or DLLs: the code is
//! position-independent and makes the NT system calls directly, the way
//! `ntdll`'s stubs would. Its UTF-16 strings are written by Rust into the
//! same section, [`STRINGS_AT`] bytes after the code's start, and reached
//! RIP-relative.
//!
//! It reports a failed step with `PolyTrace(0xBADBAD, step, status)` and
//! ends with [`FAIL_CODE`]; on success it ends with [`PASS_CODE`].
//!
//! Steps:
//! 1. `NtCreateKey` `\Registry\Machine\SOFTWARE\PolyKernelTest`: a new key.
//! 2. `NtSetValueKey`: `Name` (`REG_SZ`), `Count` (`REG_DWORD`), `Big`
//!    (`REG_QWORD`), `Blob` (`REG_BINARY`).
//! 3. `NtEnumerateValueKey` 0..3 (`KeyValueBasicInformation`) succeed, 4 is
//!    `STATUS_NO_MORE_ENTRIES`; entry 1 is `Count`, a `REG_DWORD`.
//! 4. `NtQueryValueKey` `Count` (`KeyValuePartialInformation`) reads the
//!    data back; `Name` into a 12-byte buffer is `STATUS_BUFFER_OVERFLOW`
//!    with the full size in `ResultLength`.
//! 5. `NtQueryKey` (`KeyFullInformation`) counts four values.
//! 6. A default value: `ProductName` under `SOFTWARE\Microsoft\Windows
//!    NT\CurrentVersion` is "Poly"; `ComputerName` through the
//!    `CurrentControlSet` symbolic link is there.
//! 7. `NtNotifyChangeKey` with an event: `STATUS_PENDING`; a value change
//!    writes the I/O status block and signals the event
//!    (`NtWaitForSingleObject` with a zero timeout returns at once).
//! 8. `NtDeleteKey`, every handle closed; opening the key again is
//!    `STATUS_OBJECT_NAME_NOT_FOUND`.

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_personality_nt::pe_writer::{self, PeImage, PeSection};

pub const PASS_CODE: u8 = 0x5A;
pub const FAIL_CODE: u8 = 0xEE;
pub const IMAGE_BASE: u64 = 0x1_4000_0000;

/// Where the strings start, from the start of the code.
pub const STRINGS_AT: usize = 0x2000;

/// The strings the program uses, in order.
pub const STRINGS: [&str; 10] = [
    "\\Registry\\Machine\\SOFTWARE\\PolyKernelTest",
    "\\Registry\\Machine\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
    "\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Control\\ComputerName\\ComputerName",
    "Name",
    "Count",
    "Big",
    "Blob",
    "Poly\0",
    "ProductName",
    "ComputerName",
];

/// Offset of string `i` from the code's start (each 8-byte aligned; all
/// are ASCII, so a string's UTF-16 length is twice its byte length).
const fn at(i: usize) -> usize {
    let mut o = STRINGS_AT;
    let mut j = 0;
    while j < i {
        o += (STRINGS[j].len() * 2).div_ceil(8) * 8;
        j += 1;
    }
    o
}

/// Bytes of string `i` in UTF-16.
const fn len(i: usize) -> usize {
    STRINGS[i].len() * 2
}

global_asm!(
    r#"
    .macro polyreg_ustr dst, off, len
    lea rax, [rip + poly_nt_reg_test_code + \off]
    mov [rbx + \dst + 8], rax
    mov word ptr [rbx + \dst], \len
    mov word ptr [rbx + \dst + 2], \len
    .endm

    .section .rodata.poly_nt_reg_test, "a", @progbits
    .balign 16
    .global poly_nt_reg_test_code
poly_nt_reg_test_code:
    sub rsp, 0x1000
    mov rbx, rsp
    // Scratch, from rbx: 0x200 H1, 0x208 H2, 0x210 EVT, 0x218 H3,
    // 0x220 disposition, 0x228 result length, 0x230 IO_STATUS_BLOCK,
    // 0x240 zero timeout, 0x250 key UNICODE_STRING, 0x260 value
    // UNICODE_STRING, 0x270 OBJECT_ATTRIBUTES, 0x2A0 data, 0x300 buffer.

    // ----- 1. NtCreateKey(&H1, KEY_ALL_ACCESS, &OA, 0, NULL, 0, &disp)
    mov r15d, 1
    polyreg_ustr 0x250, {s0}, {l0}
    call polyreg_oa
    lea r10, [rbx+0x200]
    mov edx, 0xF003F
    lea r8, [rbx+0x270]
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    lea rax, [rbx+0x220]
    mov [rsp+0x38], rax
    mov eax, 0x100
    syscall
    test eax, eax
    jnz polyreg_fail
    mov r15d, 2
    mov eax, [rbx+0x220]
    cmp eax, 1
    jne polyreg_fail

    // ----- 2. values
    mov r15d, 3
    polyreg_ustr 0x260, {s3}, {l3}
    mov r9d, 1
    lea rax, [rip + poly_nt_reg_test_code + {s7}]
    mov ecx, {l7}
    call polyreg_set
    mov r15d, 4
    polyreg_ustr 0x260, {s4}, {l4}
    mov dword ptr [rbx+0x2A0], 0x1234
    mov r9d, 4
    lea rax, [rbx+0x2A0]
    mov ecx, 4
    call polyreg_set
    mov r15d, 5
    polyreg_ustr 0x260, {s5}, {l5}
    mov rax, 0x1122334455667788
    mov [rbx+0x2A8], rax
    mov r9d, 11
    lea rax, [rbx+0x2A8]
    mov ecx, 8
    call polyreg_set
    mov r15d, 6
    polyreg_ustr 0x260, {s6}, {l6}
    mov r9d, 3
    lea rax, [rip + poly_nt_reg_test_code + {s0}]
    mov ecx, 6
    call polyreg_set

    // ----- 3. NtEnumerateValueKey(H1, i, KeyValueBasicInformation, BUF, 256, &RET)
    mov r15d, 7
    xor r12d, r12d
polyreg_enum:
    mov edx, r12d
    call polyreg_enum_value
    test eax, eax
    jnz polyreg_fail
    inc r12d
    cmp r12d, 4
    jne polyreg_enum
    mov r15d, 8
    mov edx, 4
    call polyreg_enum_value
    cmp eax, 0x8000001A
    jne polyreg_fail
    mov r15d, 9
    mov edx, 1
    call polyreg_enum_value
    test eax, eax
    jnz polyreg_fail
    mov eax, [rbx+0x304]
    cmp eax, 4
    jne polyreg_fail
    mov eax, [rbx+0x308]
    cmp eax, {l4}
    jne polyreg_fail

    // ----- 4. NtQueryValueKey(H1, "Count", KeyValuePartialInformation, BUF, 256, &RET)
    mov r15d, 10
    polyreg_ustr 0x260, {s4}, {l4}
    mov r10, [rbx+0x200]
    mov r9d, 256
    call polyreg_query_value
    test eax, eax
    jnz polyreg_fail
    mov r15d, 11
    mov eax, [rbx+0x304]
    cmp eax, 4
    jne polyreg_fail
    mov eax, [rbx+0x308]
    cmp eax, 4
    jne polyreg_fail
    mov eax, [rbx+0x30C]
    cmp eax, 0x1234
    jne polyreg_fail
    mov eax, [rbx+0x228]
    cmp eax, 16
    jne polyreg_fail
    // "Name" into 12 bytes: the fixed part only.
    mov r15d, 12
    polyreg_ustr 0x260, {s3}, {l3}
    mov r10, [rbx+0x200]
    mov r9d, 12
    call polyreg_query_value
    cmp eax, 0x80000005
    jne polyreg_fail
    mov r15d, 13
    mov eax, [rbx+0x228]
    cmp eax, 12 + {l7}
    jne polyreg_fail

    // ----- 5. NtQueryKey(H1, KeyFullInformation, BUF, 256, &RET)
    mov r15d, 14
    mov r10, [rbx+0x200]
    mov edx, 2
    lea r8, [rbx+0x300]
    mov r9d, 256
    lea rax, [rbx+0x228]
    mov [rsp+0x28], rax
    mov eax, 0x107
    syscall
    test eax, eax
    jnz polyreg_fail
    mov r15d, 15
    mov eax, [rbx+0x320]
    cmp eax, 4
    jne polyreg_fail

    // ----- 6. default values
    mov r15d, 16
    polyreg_ustr 0x250, {s1}, {l1}
    call polyreg_oa
    lea r10, [rbx+0x208]
    call polyreg_open
    test eax, eax
    jnz polyreg_fail
    mov r15d, 17
    polyreg_ustr 0x260, {s8}, {l8}
    mov r10, [rbx+0x208]
    mov r9d, 256
    call polyreg_query_value
    test eax, eax
    jnz polyreg_fail
    mov r15d, 18
    mov eax, [rbx+0x308]
    cmp eax, 10
    jne polyreg_fail
    mov rax, 0x0079006C006F0050
    cmp [rbx+0x30C], rax
    jne polyreg_fail
    mov r15d, 19
    polyreg_ustr 0x250, {s2}, {l2}
    call polyreg_oa
    lea r10, [rbx+0x218]
    call polyreg_open
    test eax, eax
    jnz polyreg_fail
    mov r15d, 20
    polyreg_ustr 0x260, {s9}, {l9}
    mov r10, [rbx+0x218]
    mov r9d, 256
    call polyreg_query_value
    test eax, eax
    jnz polyreg_fail

    // ----- 7. notification
    // NtCreateEvent(&EVT, EVENT_ALL_ACCESS, NULL, NotificationEvent, FALSE)
    mov r15d, 21
    lea r10, [rbx+0x210]
    mov edx, 0x1F0003
    xor r8d, r8d
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov eax, 0x0A
    syscall
    test eax, eax
    jnz polyreg_fail
    // NtNotifyChangeKey(H1, EVT, NULL, NULL, &IOSB, LAST_SET, FALSE, NULL, 0, TRUE)
    mov r15d, 22
    mov dword ptr [rbx+0x230], 0xFFFFFFFF
    mov r10, [rbx+0x200]
    mov rdx, [rbx+0x210]
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x230]
    mov [rsp+0x28], rax
    mov qword ptr [rsp+0x30], 4
    mov qword ptr [rsp+0x38], 0
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 0
    mov qword ptr [rsp+0x50], 1
    mov eax, 0x10B
    syscall
    cmp eax, 0x103
    jne polyreg_fail
    // A change: Count = 0x5678.
    mov r15d, 23
    polyreg_ustr 0x260, {s4}, {l4}
    mov dword ptr [rbx+0x2A0], 0x5678
    mov r9d, 4
    lea rax, [rbx+0x2A0]
    mov ecx, 4
    call polyreg_set
    mov r15d, 24
    mov eax, [rbx+0x230]
    test eax, eax
    jnz polyreg_fail
    // NtWaitForSingleObject(EVT, FALSE, &zero): signalled already.
    mov r15d, 25
    mov qword ptr [rbx+0x240], 0
    mov r10, [rbx+0x210]
    xor edx, edx
    lea r8, [rbx+0x240]
    mov eax, 0x2D
    syscall
    test eax, eax
    jnz polyreg_fail

    // ----- 8. delete, close, gone
    mov r15d, 26
    mov r10, [rbx+0x200]
    mov eax, 0x103
    syscall
    test eax, eax
    jnz polyreg_fail
    mov r15d, 27
    mov r10, [rbx+0x200]
    mov eax, 0
    syscall
    test eax, eax
    jnz polyreg_fail
    mov r10, [rbx+0x208]
    mov eax, 0
    syscall
    test eax, eax
    jnz polyreg_fail
    mov r10, [rbx+0x210]
    mov eax, 0
    syscall
    test eax, eax
    jnz polyreg_fail
    mov r10, [rbx+0x218]
    mov eax, 0
    syscall
    test eax, eax
    jnz polyreg_fail
    mov r15d, 28
    polyreg_ustr 0x250, {s0}, {l0}
    call polyreg_oa
    lea r10, [rbx+0x200]
    call polyreg_open
    cmp eax, 0xC0000034
    jne polyreg_fail

    // ----- success
    mov r10, 0xC0FFEE
    mov edx, 0x66
    xor r8d, r8d
    mov eax, 0x3000
    syscall
    mov r10, -1
    mov edx, {pass}
    mov eax, 0x80
    syscall
    ud2

// OBJECT_ATTRIBUTES at 0x270 naming the key UNICODE_STRING at 0x250,
// OBJ_CASE_INSENSITIVE.
polyreg_oa:
    mov dword ptr [rbx+0x270], 48
    mov qword ptr [rbx+0x278], 0
    lea rax, [rbx+0x250]
    mov [rbx+0x280], rax
    mov dword ptr [rbx+0x288], 0x40
    mov qword ptr [rbx+0x290], 0
    mov qword ptr [rbx+0x298], 0
    ret

// NtOpenKey(r10 = &handle, KEY_READ, &OA): eax = status.
polyreg_open:
    mov edx, 0x20019
    lea r8, [rbx+0x270]
    mov eax, 0x101
    syscall
    ret

// NtSetValueKey(H1, &VALUE_NAME, 0, r9d = type, rax = data, ecx = size);
// fails the step on an error.
// The stack arguments go above the helper's own rsp (the kernel reads
// them relative to rsp at the syscall).
polyreg_set:
    mov [rsp+0x28], rax
    mov [rsp+0x30], rcx
    mov r10, [rbx+0x200]
    lea rdx, [rbx+0x260]
    xor r8d, r8d
    mov eax, 0x109
    syscall
    test eax, eax
    jnz polyreg_fail
    ret

// NtEnumerateValueKey(H1, edx = index, KeyValueBasicInformation, BUF, 256, &RET).
polyreg_enum_value:
    mov r10, [rbx+0x200]
    xor r8d, r8d
    lea r9, [rbx+0x300]
    mov qword ptr [rsp+0x28], 256
    lea rax, [rbx+0x228]
    mov [rsp+0x30], rax
    mov eax, 0x106
    syscall
    ret

// NtQueryValueKey(r10 = key, &VALUE_NAME, KeyValuePartialInformation, BUF,
// r9d = length, &RET).
polyreg_query_value:
    mov [rsp+0x28], r9
    lea rax, [rbx+0x228]
    mov [rsp+0x30], rax
    lea rdx, [rbx+0x260]
    mov r8d, 2
    lea r9, [rbx+0x300]
    mov eax, 0x108
    syscall
    ret

polyreg_fail:
    // PolyTrace(0xBADBAD, step, status); then terminate with FAIL_CODE.
    mov r8d, eax
    mov rdx, r15
    mov r10, 0xBADBAD
    mov eax, 0x3000
    syscall
    mov r10, -1
    mov edx, {fail}
    mov eax, 0x80
    syscall
    ud2
    .global poly_nt_reg_test_code_end
poly_nt_reg_test_code_end:
"#,
    s0 = const at(0),
    l0 = const len(0),
    s1 = const at(1),
    l1 = const len(1),
    s2 = const at(2),
    l2 = const len(2),
    s3 = const at(3),
    l3 = const len(3),
    s4 = const at(4),
    l4 = const len(4),
    s5 = const at(5),
    l5 = const len(5),
    s6 = const at(6),
    l6 = const len(6),
    s7 = const at(7),
    l7 = const len(7),
    s8 = const at(8),
    l8 = const len(8),
    s9 = const at(9),
    l9 = const len(9),
    pass = const PASS_CODE as u32,
    fail = const FAIL_CODE as u32,
);

unsafe extern "C" {
    static poly_nt_reg_test_code: u8;
    static poly_nt_reg_test_code_end: u8;
}

/// The assembled code (position-independent).
fn code() -> &'static [u8] {
    let start = &raw const poly_nt_reg_test_code;
    let end = &raw const poly_nt_reg_test_code_end;
    // SAFETY: the two symbols bound the code in a read-only section of the
    // kernel image.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

/// The PE executable: one `.text` section, the code then the strings at
/// [`STRINGS_AT`].
pub fn image() -> Vec<u8> {
    let mut data = code().to_vec();
    assert!(data.len() <= STRINGS_AT, "the registry test code outgrew its string offset");
    data.resize(STRINGS_AT, 0xCC);
    for (i, s) in STRINGS.iter().enumerate() {
        data.resize(at(i), 0);
        data.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
    }
    let virtual_size = data.len() as u32;
    PeImage {
        image_base: IMAGE_BASE,
        entry: 0x1000,
        sections: vec![PeSection { name: *b".text\0\0\0", data, virtual_size, characteristics: pe_writer::TEXT }],
        relocations: Vec::new(),
        fixed: false,
        stack_reserve: 0x1_0000,
        stack_commit: 0x4000,
    }
    .write()
}

mod tests {
    use super::{FAIL_CODE, PASS_CODE, image};
    use crate::kernel_test;
    use crate::process::Process;
    use crate::thread;
    use crate::user::memory::Layout;
    use crate::user::{self, Memory};
    use alloc::sync::Arc;
    use poly_binfmt::{Arch, Bits, Format};
    use poly_ntobj::name::wide;

    const FORMAT: Format = Format::Pe { bits: Bits::B64, arch: Arch::X86_64 };

    kernel_test! {
        /// Build the registry test PE, run it as an NT process from ring 3,
        /// and check what it left in the registry.
        fn nt_registry_program_runs_from_ring3() -> TestResult {
            let file = image();
            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let start =
                super::super::start(&process, &memory, &file, pid).map_err(|_| "the NT loader rejected the test PE")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!("nt: the registry test ended with {code:#x}, not {PASS_CODE:#x} (fail {FAIL_CODE:#x})");
                return Err("the NT registry test program did not pass every check");
            }
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the NT process kept its memory after it exited");
            }
            let left = super::super::with(|s| {
                let reg = &s.sys.registry.reg;
                let sw = reg.find(reg.machine(), "SOFTWARE").expect("the default hive");
                (reg.child(sw, &wide("PolyKernelTest")).is_some(), s.sys.registry.pending_requests())
            });
            if left != (false, 0) {
                return Err("the test key or a notification request outlived the program");
            }
            Ok(())
        }
    }

    kernel_test! {
        /// A hive file written to a directory on the VFS (tmpfs here) comes
        /// back whole, the temporary `.new` file is gone, and a second save
        /// replaces the first.
        fn nt_registry_hive_files_round_trip() -> TestResult {
            use super::super::registry::{load_in, read_file, save_in};
            let dir = "/tmp/poly-registry-test/poly/nt/registry";
            let mut reg = poly_registry::Registry::new();
            let m = reg.machine();
            let h = reg.mount_hive(m, &wide("T"), "T", None, 0).map_err(|_| "mount")?;
            let root = reg.hive(h).root;
            reg.set_value(root, &wide("v"), poly_registry::consts::REG_DWORD, &[1, 2, 3, 4], 0).map_err(|_| "set")?;
            let first = reg.save(h);
            save_in(dir, "T", &first).map_err(|e| {
                crate::kprintln!("nt: registry test: the first save: {e:?}");
                "the first save failed"
            })?;
            reg.set_value(root, &wide("w"), poly_registry::consts::REG_SZ, &[0, 0], 0).map_err(|_| "set")?;
            let second = reg.save(h);
            save_in(dir, "T", &second).map_err(|e| {
                crate::kprintln!("nt: registry test: the second save: {e:?}");
                "the second save failed"
            })?;
            let back = load_in(dir, "T").ok_or("the hive file is not there")?;
            if back != second {
                return Err("the hive file came back different");
            }
            if read_file(&alloc::format!("{dir}/T.hive.new")).is_some() {
                return Err("the temporary file was left behind");
            }
            let file = poly_registry::hive::decode(&back).map_err(|_| "the saved file does not decode")?;
            if file.sequence != 2 || file.root.values.len() != 2 {
                return Err("the saved file holds the wrong hive");
            }
            Ok(())
        }
    }
}
