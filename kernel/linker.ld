/*
 * Kernel memory layout.
 *
 * The image is loaded at physical 1 MiB. The 32-bit boot stub (.boot*) runs
 * at its physical address with paging off; everything else is linked into
 * the top 2 GiB of the address space (KERNEL_VMA + physical address) and
 * loaded at the matching physical address via AT().
 *
 * The kernel page tables (arch/x86_64/paging.rs) map each part with its own
 * permissions, so every boundary below is page-aligned and named:
 *
 *   __text_start   .. __text_end     read-only, executable
 *   __rodata_start .. __rodata_end   read-only, no-execute (incl. .kernel_tests)
 *   __data_start   .. __data_end     read-write, no-execute (.data, .bss)
 *   one stack after another, each above an unmapped guard page
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

    __text_start = .;
    .text : AT(ADDR(.text) - KERNEL_VMA) { *(.text .text.*) } :text
    . = ALIGN(4096);
    __text_end = .;

    __rodata_start = .;
    .rodata : AT(ADDR(.rodata) - KERNEL_VMA) { *(.rodata .rodata.*) } :rodata

    /*
     * In-kernel tests: one `ktest::KernelTest` record per `kernel_test!`, each
     * in its own `.kernel_tests.<path>` input section. Sorting by name gives a
     * stable run order; KEEP stops --gc-sections dropping the unreferenced
     * records.
     */
    .kernel_tests : AT(ADDR(.kernel_tests) - KERNEL_VMA) ALIGN(8) {
        __kernel_tests_start = .;
        KEEP(*(SORT_BY_NAME(.kernel_tests.*)))
        __kernel_tests_end = .;
    } :rodata
    . = ALIGN(4096);
    __rodata_end = .;

    __data_start = .;
    .data : AT(ADDR(.data) - KERNEL_VMA) { *(.data .data.*) *(.got .got.*) } :data
    .bss (NOLOAD) : AT(ADDR(.bss) - KERNEL_VMA) ALIGN(16) { *(.bss .bss.*) *(COMMON) } :data
    . = ALIGN(4096);
    __data_end = .;

    /*
     * Kernel stacks (sizes are set where they are defined). The page below
     * each one is left unmapped, so running off the bottom of a stack faults
     * instead of overwriting whatever lies below it.
     */
    __boot_stack_guard = .;
    . += 4096;
    .stack.boot (NOLOAD) : AT(ADDR(.stack.boot) - KERNEL_VMA) {
        __boot_stack_bottom = .;
        KEEP(*(.stack.boot))
        __boot_stack_top = .;
    } :data
    . = ALIGN(4096);

    __double_fault_stack_guard = .;
    . += 4096;
    .stack.double_fault (NOLOAD) : AT(ADDR(.stack.double_fault) - KERNEL_VMA) {
        __double_fault_stack_bottom = .;
        KEEP(*(.stack.double_fault))
        __double_fault_stack_top = .;
    } :data
    . = ALIGN(4096);

    __kernel_phys_end = . - KERNEL_VMA;

    /DISCARD/ : { *(.eh_frame*) *(.note.gnu*) *(.comment*) }
}
