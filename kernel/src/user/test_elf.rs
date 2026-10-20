//! Static ELF executables for the exec tests, written byte by byte in
//! assembly: the ELF header, the program headers and the code, all in one
//! block of read-only kernel data that the tests hand to `exec` as a file.
//! No toolchain beyond the kernel's own assembler is involved, so the
//! images are the same on every machine. Layouts follow the System V gABI
//! (ELF header, program headers) with the constants of the public
//! `<elf.h>`.
//!
//! Each image has three or four program headers:
//!
//! | Segment | File | Memory | Flags |
//! |---|---|---|---|
//! | text | offset 0 to the end of the code (headers included, so it maps the program header table) | [`TEXT_BASE`] + offset | R X |
//! | data | 16 bytes of initialised data | [`DATA_DELTA`] above the text, + its file offset | R W |
//! | `.bss` | | [`BSS`] bytes after the data, two pages and more | R W |
//! | `PT_INTERP` (only [`TestElf::Interp`]) | a path | | |
//! | `PT_GNU_STACK` | | | R W |
//!
//! After the data come 64 bytes of `0xaa` that belong to no segment, and the
//! text's last page holds the data's bytes in the file: a loader that maps
//! a segment's last page without zeroing the rest shows them.
//!
//! The program ([`TestElf::Exec`] at fixed addresses, [`TestElf::Pie`] as a
//! static position-independent executable; the code is the same) reports
//! what it finds through system calls a test hook understands, with
//! [`REPORT`] in RAX, the [`Seen`] kind in RDI and two values in RSI and
//! RDX, then exits with [`EXIT_CODE`] through [`EXIT`]:
//!
//! 1. its initial RSP, `argc`, every `argv[i]` pointer, `argv[argc]`, every
//!    `envp[i]` pointer and every auxiliary vector entry including
//!    `AT_NULL`, walking the stack as the psABI §3.4.1 lays it out;
//! 2. the first data word, the 8 bytes after the data (`p_filesz`) and the
//!    last 8 bytes of the data's file page (both must read 0);
//! 3. a `.bss` word two pages further (0), then the same word after
//!    writing it;
//! 4. the data word after writing it (the file must not change);
//! 5. a word written 192 KiB below the initial RSP, past the initial stack
//!    region: the stack grows down on demand.

use core::arch::global_asm;

/// System call: report RDI = kind, RSI and RDX = values.
pub const REPORT: u64 = 1;
/// System call: exit the thread with RDI.
pub const EXIT: u64 = 2;
/// What the program exits with when it got to the end.
pub const EXIT_CODE: usize = 0x600d;

/// Link address of the text segment of [`TestElf::Exec`] (0 for the PIE).
pub const TEXT_BASE: u64 = 0x40_0000;
/// Distance from the text's link base to the data's.
pub const DATA_DELTA: u64 = 0x20_0000;
/// `p_memsz - p_filesz` of the data segment.
pub const BSS: u64 = 0x2800;
/// Offset from the data at which the program tests `.bss`.
pub const BSS_PROBE: u64 = 0x2000;
/// How far below its initial RSP the program writes.
pub const STACK_PROBE: u64 = 0x3_0000;
/// The first data word in the file.
pub const DATA_WORD: u64 = 0x1122_3344_5566_7788;
/// What the program writes into the data word, the `.bss` word and the
/// stack.
pub const DATA_WRITE: u64 = 0x77;
pub const BSS_WRITE: u64 = 0x5eed;
pub const STACK_WRITE: u64 = 0x57ac;

/// What a [`REPORT`] call reports (RDI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Seen {
    /// RSI = the initial RSP.
    Rsp = 1,
    /// RSI = `argc`.
    Argc = 2,
    /// RSI = i, RDX = `argv[i]`.
    Arg = 3,
    /// RSI = `argc`, RDX = `argv[argc]` (must be 0).
    ArgvEnd = 4,
    /// RSI = i, RDX = `envp[i]`.
    Env = 5,
    /// RSI = type, RDX = value of one auxiliary vector entry.
    Aux = 6,
    /// RSI = the data word, RDX = 0 before writing it, 1 after.
    Data = 7,
    /// RSI = 8 bytes that must be 0: RDX = 0 just past `p_filesz`, 1 at
    /// the end of that page.
    Tail = 8,
    /// RSI = the `.bss` word, RDX = 0 before writing it, 1 after.
    Bss = 9,
    /// RSI = the word written below the stack.
    Stack = 10,
}

global_asm!(
    r#"
    .section .rodata.user_elf, "a", @progbits

.macro poly_elf_report kind, a, b
    mov $\kind, %edi
    mov \a, %rsi
    mov \b, %rdx
    mov ${report}, %eax
    syscall
.endm

.macro poly_test_elf name, etype, tbase, interp
    .balign 16
    .global \name\()_start
\name\()_start:
    /* Elf64_Ehdr: e_ident (ELFCLASS64, ELFDATA2LSB, EV_CURRENT,
       ELFOSABI_SYSV), e_type, e_machine (EM_X86_64), e_version, e_entry,
       e_phoff, e_shoff, e_flags, e_ehsize, e_phentsize, e_phnum,
       e_shentsize, e_shnum, e_shstrndx. */
    .byte 0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0
    .quad 0
    .short \etype, 62
    .long 1
    .quad \tbase + (\name\()_entry - \name\()_start)
    .quad 64, 0
    .long 0
    .short 64, 56, 3 + \interp, 64, 0, 0

    /* Elf64_Phdr: p_type, p_flags, p_offset, p_vaddr, p_paddr, p_filesz,
       p_memsz, p_align. Text: PT_LOAD, PF_R | PF_X. */
    .long 1, 5
    .quad 0, \tbase, \tbase
    .quad \name\()_text_end - \name\()_start, \name\()_text_end - \name\()_start
    .quad 0x1000
    /* Data and .bss: PT_LOAD, PF_R | PF_W. */
    .long 1, 6
    .quad \name\()_data - \name\()_start
    .quad \tbase + {delta} + (\name\()_data - \name\()_start)
    .quad \tbase + {delta} + (\name\()_data - \name\()_start)
    .quad \name\()_data_end - \name\()_data
    .quad \name\()_data_end - \name\()_data + {bss}
    .quad 0x1000
    .if \interp
    /* PT_INTERP, PF_R. */
    .long 3, 4
    .quad \name\()_interp - \name\()_start
    .quad \tbase + (\name\()_interp - \name\()_start), \tbase + (\name\()_interp - \name\()_start)
    .quad \name\()_interp_end - \name\()_interp, \name\()_interp_end - \name\()_interp
    .quad 1
    .endif
    /* PT_GNU_STACK, PF_R | PF_W: a non-executable stack. */
    .long 0x6474e551, 6
    .quad 0, 0, 0, 0, 0, 16

\name\()_entry:
    mov %rsp, %r12
    poly_elf_report {rsp}, %r12, $0
    mov (%r12), %rbx
    poly_elf_report {argc}, %rbx, $0
    lea 8(%r12), %r13
    xor %r14d, %r14d
1:
    cmp %rbx, %r14
    je 2f
    mov (%r13,%r14,8), %r15
    poly_elf_report {arg}, %r14, %r15
    inc %r14
    jmp 1b
2:
    mov (%r13,%rbx,8), %r15
    poly_elf_report {argv_end}, %rbx, %r15
    lea 8(%r13,%rbx,8), %r13
    xor %r14d, %r14d
3:
    mov (%r13), %r15
    test %r15, %r15
    jz 4f
    poly_elf_report {env}, %r14, %r15
    add $8, %r13
    inc %r14
    jmp 3b
4:
    add $8, %r13
5:
    mov (%r13), %r14
    mov 8(%r13), %r15
    poly_elf_report {aux}, %r14, %r15
    add $16, %r13
    test %r14, %r14
    jnz 5b

    /* RBP = the data word: the image's start, wherever it was loaded,
       plus the data's distance from it in memory. */
    lea \name\()_start(%rip), %rbp
    add $({delta} + \name\()_data - \name\()_start), %rbp
    mov (%rbp), %r14
    poly_elf_report {data}, %r14, $0
    mov (\name\()_data_end - \name\()_data)(%rbp), %r14
    poly_elf_report {tail}, %r14, $0
    mov %rbp, %r14
    or $0xfff, %r14
    mov -7(%r14), %r15
    poly_elf_report {tail}, %r15, $1
    mov {bss_probe}(%rbp), %r14
    poly_elf_report {bss_kind}, %r14, $0
    movq ${bss_write}, {bss_probe}(%rbp)
    mov {bss_probe}(%rbp), %r14
    poly_elf_report {bss_kind}, %r14, $1
    movq ${data_write}, (%rbp)
    mov (%rbp), %r14
    poly_elf_report {data}, %r14, $1
    movq ${stack_write}, -{stack_probe}(%r12)
    mov -{stack_probe}(%r12), %r14
    poly_elf_report {stack}, %r14, $0
    mov ${exit_code}, %edi
    mov ${exit}, %eax
    syscall
    ud2
    .if \interp
\name\()_interp:
    .asciz "/lib/ld-poly.so.1"
\name\()_interp_end:
    .endif
\name\()_text_end:
    .balign 8, 0xcc
\name\()_data:
    .quad {data_word}
    .quad 0x0123456789abcdef
\name\()_data_end:
    .fill 64, 1, 0xaa
    .global \name\()_end
\name\()_end:
.endm

    poly_test_elf poly_test_elf_exec, 2, {text_base}, 0
    poly_test_elf poly_test_elf_pie, 3, 0, 0
    poly_test_elf poly_test_elf_interp, 2, {text_base}, 1
"#,
    report = const REPORT,
    exit = const EXIT,
    exit_code = const EXIT_CODE,
    text_base = const TEXT_BASE,
    delta = const DATA_DELTA,
    bss = const BSS,
    bss_probe = const BSS_PROBE,
    stack_probe = const STACK_PROBE,
    data_word = const DATA_WORD,
    data_write = const DATA_WRITE,
    bss_write = const BSS_WRITE,
    stack_write = const STACK_WRITE,
    rsp = const Seen::Rsp as u64,
    argc = const Seen::Argc as u64,
    arg = const Seen::Arg as u64,
    argv_end = const Seen::ArgvEnd as u64,
    env = const Seen::Env as u64,
    aux = const Seen::Aux as u64,
    data = const Seen::Data as u64,
    tail = const Seen::Tail as u64,
    bss_kind = const Seen::Bss as u64,
    stack = const Seen::Stack as u64,
    options(att_syntax)
);

unsafe extern "C" {
    static poly_test_elf_exec_start: u8;
    static poly_test_elf_exec_end: u8;
    static poly_test_elf_pie_start: u8;
    static poly_test_elf_pie_end: u8;
    static poly_test_elf_interp_start: u8;
    static poly_test_elf_interp_end: u8;
}

/// The test executables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestElf {
    /// `ET_EXEC` at [`TEXT_BASE`].
    Exec,
    /// `ET_DYN` without an interpreter (a static PIE), linked at 0.
    Pie,
    /// `ET_EXEC` with a `PT_INTERP`: exec must refuse it.
    Interp,
}

impl TestElf {
    /// The whole file.
    pub fn image(self) -> &'static [u8] {
        let (start, end) = match self {
            TestElf::Exec => (&raw const poly_test_elf_exec_start, &raw const poly_test_elf_exec_end),
            TestElf::Pie => (&raw const poly_test_elf_pie_start, &raw const poly_test_elf_pie_end),
            TestElf::Interp => (&raw const poly_test_elf_interp_start, &raw const poly_test_elf_interp_end),
        };
        // SAFETY: the assembly above puts each image between its two
        // symbols, in a read-only section of the kernel image.
        unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
    }

    /// Number of program headers.
    pub fn phnum(self) -> u64 {
        if self == TestElf::Interp { 4 } else { 3 }
    }
}
