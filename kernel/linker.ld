/*
 * Kernel memory layout.
 *
 * The image is loaded at physical 1 MiB. The 32-bit boot stub (.boot*) runs
 * at its physical address with paging off; everything else is linked into
 * the top 2 GiB of the address space (KERNEL_VMA + physical address) and
 * loaded at the matching physical address via AT().
 */
ENTRY(pvh_start32)

KERNEL_VMA = 0xFFFFFFFF80000000;

PHDRS
{
    boot   PT_LOAD FLAGS(7);
    text   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
    note   PT_NOTE FLAGS(4);
}

SECTIONS
{
    . = 0x100000;
    __kernel_phys_start = .;

    .boot : {
        KEEP(*(.boot.text))
        KEEP(*(.boot.data))
    } :boot

    /* PVH entry point note, read by QEMU / Xen / Firecracker loaders. */
    .note.Xen : ALIGN(4) { KEEP(*(.note.Xen)) } :boot :note

    .boot.bss (NOLOAD) : ALIGN(4096) { *(.boot.bss) } :boot

    . = ALIGN(4096);
    . += KERNEL_VMA;

    .text : AT(ADDR(.text) - KERNEL_VMA) { *(.text .text.*) } :text

    . = ALIGN(4096);
    .rodata : AT(ADDR(.rodata) - KERNEL_VMA) { *(.rodata .rodata.*) } :rodata

    . = ALIGN(4096);
    .data : AT(ADDR(.data) - KERNEL_VMA) { *(.data .data.*) *(.got .got.*) } :data
    .bss (NOLOAD) : AT(ADDR(.bss) - KERNEL_VMA) ALIGN(16) { *(.bss .bss.*) *(COMMON) } :data

    . = ALIGN(4096);
    __kernel_phys_end = . - KERNEL_VMA;

    /DISCARD/ : { *(.eh_frame*) *(.note.gnu*) *(.comment*) }
}
