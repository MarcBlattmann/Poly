//! A self-assembled ring-3 PE that checks the Windows file system layout
//! from inside an NT process (ADR 0072). No Microsoft toolchain or DLLs:
//! the code is position-independent and makes the NT system calls the way
//! `ntdll`'s stubs would (ADR 0063). Every label is prefixed `ntlayout_`
//! (all `global_asm!` blocks share one object).
//!
//! It starts with the PEB in RCX (the thread trampoline passes it, ADR
//! 0065), reports a failed step with `PolyTrace(0xBADBAD, step, status)`
//! and ends with [`FAIL_CODE`], else with [`PASS_CODE`]:
//!
//! 1. `PEB.ProcessParameters` (PEB + 0x20) is set, its `ImagePathName` is
//!    not empty and it has an environment block (+ 0x80, ADR 0072).
//! 2. The block has `TEMP=`; its value becomes the NT path `\??\<TEMP>`.
//! 3. `NtOpenFile` of `\??\c:\windows\SYSTEM32` as a directory (any case).
//! 4. `C:\Windows` enumerated with `NtQueryDirectoryFile` and the pattern
//!    `system32` finds one directory, named `System32` in its own case.
//! 5. The TEMP directory opens; a file `ntlayout.tmp` is created relative
//!    to its handle, 8 bytes are written, and an enumeration of the
//!    directory with `NTLAYOUT.TMP` finds it with its size.
//! 6. `NtDeleteFile` removes it; opening it again is
//!    `STATUS_OBJECT_NAME_NOT_FOUND`.

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_personality_nt::pe_writer::{self, PeImage, PeSection};

pub const PASS_CODE: u8 = 0x5A;
pub const FAIL_CODE: u8 = 0xEE;
pub const IMAGE_BASE: u64 = 0x1_4000_0000;

global_asm!(
    r#"
    .section .rodata.poly_nt_layout, "a", @progbits
    .balign 16
    .global poly_nt_layout_code
poly_nt_layout_code:
    mov r14, rcx
    sub rsp, 0x3000
    mov rbx, rsp

    // ----- 1. the process parameters -------------------------------------
    mov r15d, 1
    xor eax, eax
    mov rsi, [r14+0x20]
    test rsi, rsi
    jz ntlayout_fail
    cmp word ptr [rsi+0x60], 0
    je ntlayout_fail
    mov rdi, [rsi+0x80]
    test rdi, rdi
    jz ntlayout_fail

    // ----- 2. TEMP= in the environment block -----------------------------
    mov r15d, 2
    mov rcx, 0x0050004D00450054
ntlayout_scan:
    cmp word ptr [rdi], 0
    je ntlayout_fail
    cmp [rdi], rcx
    jne ntlayout_next
    cmp word ptr [rdi+8], 0x3D
    je ntlayout_found
ntlayout_next:
    add rdi, 2
    cmp word ptr [rdi-2], 0
    jne ntlayout_next
    jmp ntlayout_scan
ntlayout_found:
    lea r12, [rdi+10]
    xor r13d, r13d
ntlayout_len:
    cmp word ptr [r12+r13*2], 0
    je ntlayout_len_done
    inc r13
    jmp ntlayout_len
ntlayout_len_done:
    test r13, r13
    jz ntlayout_fail
    // "\??\" + the value at [rbx+0x800]; its length in bytes at [rbx+0x7F0].
    mov rax, 0x005C003F003F005C
    mov [rbx+0x800], rax
    lea rdi, [rbx+0x808]
    mov rsi, r12
    mov rcx, r13
    rep movsw
    lea rax, [r13+4]
    shl eax, 1
    mov [rbx+0x7F0], ax

    // ----- 3. open System32 in another case --------------------------------
    mov r15d, 3
    lea rax, [rip+ntlayout_sys32]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 46
    mov word ptr [rbx+0x152], 46
    call ntlayout_oa
    call ntlayout_open_dir
    test eax, eax
    jnz ntlayout_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    // ----- 4. enumerate C:\Windows for system32 ----------------------------
    mov r15d, 4
    lea rax, [rip+ntlayout_windows]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 28
    mov word ptr [rbx+0x152], 28
    call ntlayout_oa
    call ntlayout_open_dir
    test eax, eax
    jnz ntlayout_fail
    mov r15d, 5
    lea rax, [rip+ntlayout_pat_sys32]
    mov [rbx+0x1C8], rax
    mov word ptr [rbx+0x1C0], 16
    mov word ptr [rbx+0x1C2], 16
    call ntlayout_query
    test eax, eax
    jnz ntlayout_fail
    // FILE_DIRECTORY_INFORMATION: FileAttributes at 56, FileNameLength at
    // 60, FileName at 64.
    mov r15d, 6
    cmp dword ptr [rbx+0x400+60], 16
    jne ntlayout_fail
    test dword ptr [rbx+0x400+56], 0x10
    jz ntlayout_fail
    mov rax, [rbx+0x400+64]
    cmp rax, [rip+ntlayout_name_sys32]
    jne ntlayout_fail
    mov rax, [rbx+0x400+72]
    cmp rax, [rip+ntlayout_name_sys32+8]
    jne ntlayout_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    // ----- 5. the TEMP directory: create, write, find ------------------------
    mov r15d, 7
    lea rax, [rbx+0x800]
    mov [rbx+0x158], rax
    mov ax, [rbx+0x7F0]
    mov [rbx+0x150], ax
    mov [rbx+0x152], ax
    call ntlayout_oa
    call ntlayout_open_dir
    test eax, eax
    jnz ntlayout_fail
    mov rax, [rbx+0x100]
    mov [rbx+0x108], rax

    mov r15d, 8
    call ntlayout_tmp_oa
    // NtCreateFile(&h, FILE_ALL_ACCESS, &oa, &iosb, 0, FILE_ATTRIBUTE_NORMAL,
    //   SHARE_ALL, FILE_CREATE, SYNC|NON_DIR, 0, 0)
    lea r10, [rbx+0x100]
    mov edx, 0x1F01FF
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0x80
    mov qword ptr [rsp+0x38], 7
    mov qword ptr [rsp+0x40], 2
    mov qword ptr [rsp+0x48], 0x60
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0xC0
    syscall
    test eax, eax
    jnz ntlayout_fail

    mov r15d, 9
    mov rax, 0x706D6554796C6F50
    mov [rbx+0x190], rax
    mov r10, [rbx+0x100]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x190]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 8
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 0
    mov eax, 0xC3
    syscall
    test eax, eax
    jnz ntlayout_fail
    cmp qword ptr [rbx+0x168], 8
    jne ntlayout_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    mov r15d, 10
    mov rax, [rbx+0x108]
    mov [rbx+0x100], rax
    lea rax, [rip+ntlayout_tmp_uc]
    mov [rbx+0x1C8], rax
    mov word ptr [rbx+0x1C0], 24
    mov word ptr [rbx+0x1C2], 24
    call ntlayout_query
    test eax, eax
    jnz ntlayout_fail
    mov r15d, 11
    cmp dword ptr [rbx+0x400+60], 24
    jne ntlayout_fail
    cmp qword ptr [rbx+0x400+40], 8
    jne ntlayout_fail
    mov rax, [rbx+0x400+64]
    cmp rax, [rip+ntlayout_tmp]
    jne ntlayout_fail

    // ----- 6. delete it ------------------------------------------------------
    mov r15d, 12
    call ntlayout_tmp_oa
    lea r10, [rbx+0x120]
    mov eax, 0xCB
    syscall
    test eax, eax
    jnz ntlayout_fail
    mov r15d, 13
    call ntlayout_tmp_oa
    // NtOpenFile(&h, FILE_GENERIC_READ, &oa, &iosb, SHARE_ALL, SYNC|NON_DIR)
    lea r10, [rbx+0x100]
    mov edx, 0x120089
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x60
    mov eax, 0xC1
    syscall
    cmp eax, 0xC0000034
    jne ntlayout_fail
    mov r10, [rbx+0x108]
    xor eax, eax
    syscall

    // ----- success -------------------------------------------------------
    mov r10, -1
    mov edx, {pass}
    mov eax, 0x80
    syscall
    ud2

    // ntlayout_oa: OBJECT_ATTRIBUTES at [rbx+0x120], name at [rbx+0x150],
    // no root directory, OBJ_CASE_INSENSITIVE.
ntlayout_oa:
    mov dword ptr [rbx+0x120], 48
    mov qword ptr [rbx+0x128], 0
    lea rax, [rbx+0x150]
    mov [rbx+0x130], rax
    mov dword ptr [rbx+0x138], 0x40
    mov qword ptr [rbx+0x140], 0
    mov qword ptr [rbx+0x148], 0
    ret

    // ntlayout_tmp_oa: "ntlayout.tmp" relative to the TEMP handle [rbx+0x108].
ntlayout_tmp_oa:
    lea rax, [rip+ntlayout_tmp]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 24
    mov word ptr [rbx+0x152], 24
    call ntlayout_oa
    mov rax, [rbx+0x108]
    mov [rbx+0x128], rax
    ret

    // ntlayout_open_dir: NtOpenFile(&[rbx+0x100], LIST_DIRECTORY|SYNCHRONIZE,
    // &oa, &iosb, SHARE_ALL, DIRECTORY_FILE|SYNC); status in eax.
ntlayout_open_dir:
    lea r10, [rbx+0x100]
    mov edx, 0x100001
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x21
    mov eax, 0xC1
    syscall
    ret

    // ntlayout_query: NtQueryDirectoryFile([rbx+0x100], 0, 0, 0, &iosb,
    // [rbx+0x400], 0x200, FileDirectoryInformation, single, &[rbx+0x1C0],
    // restart); status in eax.
ntlayout_query:
    mov r10, [rbx+0x100]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x400]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 0x200
    mov qword ptr [rsp+0x40], 1
    mov qword ptr [rsp+0x48], 1
    lea rax, [rbx+0x1C0]
    mov [rsp+0x50], rax
    mov qword ptr [rsp+0x58], 1
    mov eax, 0xC6
    syscall
    ret

ntlayout_fail:
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

    .balign 8
ntlayout_name_sys32:
    .2byte 'S', 'y', 's', 't', 'e', 'm', '3', '2'
ntlayout_tmp:
    .2byte 'n', 't', 'l', 'a', 'y', 'o', 'u', 't', '.', 't', 'm', 'p'
ntlayout_tmp_uc:
    .2byte 'N', 'T', 'L', 'A', 'Y', 'O', 'U', 'T', '.', 'T', 'M', 'P'
ntlayout_pat_sys32:
    .2byte 's', 'y', 's', 't', 'e', 'm', '3', '2'
ntlayout_sys32:
    .2byte 0x5c, '?', '?', 0x5c, 'c', ':', 0x5c, 'w', 'i', 'n'
    .2byte 'd', 'o', 'w', 's', 0x5c, 'S', 'Y', 'S', 'T', 'E'
    .2byte 'M', '3', '2'
ntlayout_windows:
    .2byte 0x5c, '?', '?', 0x5c, 'C', ':', 0x5c, 'W', 'i', 'n'
    .2byte 'd', 'o', 'w', 's'
    .global poly_nt_layout_code_end
poly_nt_layout_code_end:
"#,
    pass = const PASS_CODE as u32,
    fail = const FAIL_CODE as u32,
);

unsafe extern "C" {
    static poly_nt_layout_code: u8;
    static poly_nt_layout_code_end: u8;
}

fn code() -> &'static [u8] {
    let start = &raw const poly_nt_layout_code;
    let end = &raw const poly_nt_layout_code_end;
    // SAFETY: the two symbols bound the code in a read-only section of the
    // kernel image.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

/// The PE executable: one `.text` section, relocatable, a 128 KiB stack.
pub fn image() -> Vec<u8> {
    let data = code().to_vec();
    let virtual_size = data.len() as u32;
    PeImage {
        image_base: IMAGE_BASE,
        entry: 0x1000,
        sections: vec![PeSection { name: *b".text\0\0\0", data, virtual_size, characteristics: pe_writer::TEXT }],
        relocations: Vec::new(),
        fixed: false,
        stack_reserve: 0x2_0000,
        stack_commit: 0x8000,
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
    use poly_personality_nt::layout::{self, POLY_USER, env};
    use poly_vfs::{At, Cred};

    const FORMAT: Format = Format::Pe { bits: Bits::B64, arch: Arch::X86_64 };

    fn exists(path: &str) -> bool {
        let vfs = crate::fs::vfs();
        vfs.root_context().is_ok_and(|ctx| vfs.lookup(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), true).is_ok())
    }

    kernel_test! {
        /// The Windows layout from ring 3 (ADR 0072): an NT process reads
        /// TEMP from its PEB's environment, opens `C:\Windows\System32`,
        /// enumerates `C:\Windows`, and creates, finds and deletes a file
        /// in its TEMP directory. Then the kernel checks the folders, the
        /// synced registry and that the file is gone.
        fn nt_windows_layout_from_ring3() -> TestResult {
            if crate::fs::vfs().root_context().is_err() {
                return Err("no VFS for the NT layout");
            }
            let file = image();
            let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
            let pid = crate::process::alloc_pid();
            let process = Arc::new(Process::new_nt(pid, FORMAT, false));
            let launch = super::super::Launch::of(b"/nt/c/Program Files/Poly Test/layout.exe", &[b"layout.exe", b"one arg"]);
            if launch.image_path != "C:\\Program Files\\Poly Test\\layout.exe" {
                return Err("an executable below /nt/c was not given its C: path");
            }
            let start = super::super::start_with(&process, &memory, &file, pid, &launch)
                .map_err(|_| "the NT loader rejected the test PE")?;
            let tid =
                user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
                    .map_err(|_| "could not start the NT thread")?;
            let code = thread::join(tid).map_err(|_| "join failed")?;
            if code != PASS_CODE as usize {
                crate::kprintln!("nt: the layout test ended with {code:#x}, not {PASS_CODE:#x} (fail {FAIL_CODE:#x})");
                return Err("the NT layout test program did not pass every check");
            }
            if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
                return Err("the NT process kept its memory after it exited");
            }
            for f in layout::FOLDERS {
                let p = alloc::format!("{}/{}", super::super::layout::C_PATH, f.below_drive(&POLY_USER).replace('\\', "/"));
                if !exists(&p) {
                    crate::kprintln!("nt: layout test: {p} is missing");
                    return Err("a layout folder is missing");
                }
            }
            let temp = alloc::format!("/nt/c/Users/{}/AppData/Local/Temp/ntlayout.tmp", POLY_USER.name);
            if exists(&temp) {
                return Err("the test file outlived NtDeleteFile");
            }
            let synced = super::super::with(|s| {
                let reg = &s.sys.registry.reg;
                let me = alloc::format!("{}\\{}", env::PROFILE_LIST, POLY_USER.sid);
                env::string_value(reg, reg.machine(), &me, "ProfileImagePath").map(|(_, v)| v)
            });
            if synced.as_deref() != Some("C:\\Users\\poly") {
                return Err("ProfileImagePath was not synced to the layout");
            }
            Ok(())
        }
    }
}
