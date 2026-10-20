/* The vDSO's layout (ADR 0044): one loadable segment, read and execute,
 * starting with the ELF header at address 0, holding the dynamic linking
 * tables, read-only data, unwind tables and code. Nothing writable: the
 * image is never relocated (it is position independent and uses no GOT,
 * PLT or data), so the kernel maps the same bytes into every process.
 * Writable data is discarded, which makes the link fail if the code ever
 * refers to some; a GOT or PLT would need relocations, which build.rs
 * refuses. */

PHDRS
{
    text PT_LOAD FILEHDR PHDRS FLAGS(5);          /* PF_R | PF_X */
    dynamic PT_DYNAMIC FLAGS(4);                  /* PF_R */
    eh_frame_hdr PT_GNU_EH_FRAME FLAGS(4);
}

SECTIONS
{
    . = SIZEOF_HEADERS;

    .hash           : { *(.hash) }                    :text
    .gnu.hash       : { *(.gnu.hash) }                :text
    .dynsym         : { *(.dynsym) }                  :text
    .dynstr         : { *(.dynstr) }                  :text
    .gnu.version    : { *(.gnu.version) }             :text
    .gnu.version_d  : { *(.gnu.version_d) }           :text
    .gnu.version_r  : { *(.gnu.version_r) }           :text
    .dynamic        : { *(.dynamic) }                 :text :dynamic
    .rodata         : { *(.rodata .rodata.*) }        :text
    .eh_frame_hdr   : { *(.eh_frame_hdr) }            :text :eh_frame_hdr
    .eh_frame       : { KEEP(*(.eh_frame)) }          :text

    . = ALIGN(16);
    .text           : { *(.text .text.*) }            :text

    /DISCARD/ : {
        *(.data .data.* .bss .bss.* .tbss .tdata)
        *(.comment .note.*)
    }
}
