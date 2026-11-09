/*
 * Memory layout of the aarch64 boot stub (ADR 0154).
 *
 * QEMU's `virt` machine has RAM from 0x4000_0000 and, for a guest that is not
 * a Linux `Image`, puts the devicetree blob at the start of RAM (QEMU
 * documentation, "Arm Virt"). The stub is linked 2 MiB above that, so the
 * blob (well under 2 MiB) stays intact while the stub runs.
 *
 * Identity mapped, physical address = virtual address.
 */
ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
}

SECTIONS
{
    . = 0x40200000;
    __image_start = .;

    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    } :text

    . = ALIGN(4096);
    .rodata : { *(.rodata .rodata.*) } :rodata

    . = ALIGN(4096);
    .data : { *(.data .data.*) *(.got .got.*) } :data

    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    } :data

    /* The boot CPU's stack, above everything else, 16-byte aligned. */
    .stack (NOLOAD) : ALIGN(4096) {
        . += 0x10000;
        __stack_top = .;
    } :data

    . = ALIGN(4096);
    __image_end = .;

    /DISCARD/ : { *(.comment) *(.eh_frame .eh_frame_hdr) }
}
