//! PVH entry: from 32-bit protected mode to Rust in 64-bit long mode.
//!
//! The PVH boot ABI (Xen public `arch-x86/hvm/start_info.h`, also used by
//! QEMU `-kernel` and Firecracker) enters at the address in the
//! `XEN_ELFNOTE_PHYS32_ENTRY` note with paging off, flat 32-bit segments,
//! interrupts off and `ebx` pointing at an `hvm_start_info` struct.
//!
//! The stub builds a temporary page table that maps physical [0, 1 GiB) twice:
//! at 0 (identity, so the stub keeps running after paging turns on) and at
//! [`KERNEL_VMA`](super::KERNEL_VMA) (where Rust code is linked).
//! [`paging::init`](super::paging::init) replaces both with the kernel page
//! tables early in `kernel_entry`; nothing runs at the identity-mapped
//! addresses after that, and the call below never returns.

use core::arch::global_asm;

global_asm!(
    r#"
    .section .note.Xen, "a", @note
    .balign 4
    .long 4                 /* namesz */
    .long 8                 /* descsz */
    .long 18                /* XEN_ELFNOTE_PHYS32_ENTRY */
    .asciz "Xen"
    .balign 4
    .quad pvh_start32
    .balign 4

    .section .boot.text, "ax", @progbits
    .code32
    .global pvh_start32
pvh_start32:
    cli
    cld
    mov %ebx, %esi              /* start_info; cpuid clobbers ebx */

    /* Refuse to continue without long mode. */
    mov $0x80000000, %eax
    cpuid
    cmp $0x80000001, %eax
    jb .Lno_long_mode
    mov $0x80000001, %eax
    cpuid
    bt $29, %edx
    jnc .Lno_long_mode

    /* PML4[0] -> low PDPT, PML4[511] -> high PDPT */
    mov $boot_pdpt_low, %eax
    or $0x3, %eax
    mov %eax, boot_pml4
    mov $boot_pdpt_high, %eax
    or $0x3, %eax
    mov %eax, boot_pml4 + 511 * 8

    /* low PDPT[0] and high PDPT[510] -> one PD of 512 x 2 MiB pages */
    mov $boot_pd, %eax
    or $0x3, %eax
    mov %eax, boot_pdpt_low
    mov %eax, boot_pdpt_high + 510 * 8
    xor %ecx, %ecx
1:
    mov %ecx, %eax
    shl $21, %eax
    or $0x83, %eax              /* present | writable | huge */
    mov %eax, boot_pd(, %ecx, 8)
    inc %ecx
    cmp $512, %ecx
    jne 1b

    mov $boot_pml4, %eax
    mov %eax, %cr3
    mov %cr4, %eax
    or $0x20, %eax              /* PAE */
    mov %eax, %cr4
    mov $0xC0000080, %ecx       /* EFER */
    rdmsr
    or $0x100, %eax             /* LME */
    wrmsr
    mov %cr0, %eax
    or $0x80010001, %eax        /* PG | WP | PE */
    mov %eax, %cr0

    lgdt boot_gdt_ptr
    ljmp $0x08, $pvh_start64

.Lno_long_mode:
    hlt
    jmp .Lno_long_mode

    .code64
pvh_start64:
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    xor %ax, %ax
    mov %ax, %fs
    mov %ax, %gs
    movabs $__boot_stack_top, %rsp
    xor %ebp, %ebp
    mov %esi, %edi              /* arg 0: start_info physical address */
    movabs $kernel_entry, %rax
    call *%rax
2:
    cli
    hlt
    jmp 2b

    .section .boot.data, "aw", @progbits
    .balign 8
boot_gdt:
    .quad 0
    .quad 0x00AF9A000000FFFF    /* 0x08: 64-bit code */
    .quad 0x00CF92000000FFFF    /* 0x10: data */
boot_gdt_end:
boot_gdt_ptr:
    .word boot_gdt_end - boot_gdt - 1
    .long boot_gdt

    .section .boot.bss, "aw", @nobits
    .balign 4096
boot_pml4:      .skip 4096
boot_pdpt_low:  .skip 4096
boot_pdpt_high: .skip 4096
boot_pd:        .skip 4096

    /* The linker script puts this above a guard page and defines
       __boot_stack_bottom / __boot_stack_top around it. */
    .section .stack.boot, "aw", @nobits
    .balign 4096
    .skip 64 * 1024
"#,
    options(att_syntax)
);
