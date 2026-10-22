/* EFAULT: the 16-byte user_desc of a write must be readable in full
 * (NULL, a low address, a non-canonical address, a struct running into
 * an unmapped page, a PROT_NONE page); a read-only struct is fine. A
 * read (func 0, once an LDT exists; func 2 always) needs all `count`
 * bytes (clamped to 65536 / 128) writable: a read-only, unmapped or
 * partly unmapped buffer is EFAULT, while a buffer that is only too
 * short for an unclamped count works. A failed write changes nothing.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_modify_ldt 154
/* asm/ldt.h */
#define LDT_ENTRIES 8192
#define LDT_ENTRY_SIZE 8

/* struct user_desc (asm/ldt.h), 16 bytes on x86-64. The bit fields of
 * the last word, lowest bit first: seg_32bit, contents (2 bits),
 * read_exec_only, limit_in_pages, seg_not_present, useable, lm. */
struct user_desc {
    u32 entry_number;
    u32 base_addr;
    u32 limit;
    u32 flags;
};
#define UD_SEG32 0x01
#define UD_CONTENTS(c) ((u32)(c) << 1)
#define UD_RO 0x08
#define UD_PAGES 0x10
#define UD_NP 0x20
#define UD_USEABLE 0x40
#define UD_LM 0x80

/* The raw value of %rax after the call. */
static A_UNUSED i64 mldt(i64 func, const void *ptr, u64 count) { return syscall3(SYS_modify_ldt, func, ptr, count); }
/* modify_ldt's result is a 32-bit int: an error -e arrives in %rax as
 * the 32-bit two's complement of e, zero-extended. */
#define E32(e) ((i64)(u32)(-(e)))

static A_UNUSED i64 ldt_set(int func, u32 entry, u32 base, u32 limit, u32 flags) {
    struct user_desc u = { entry, base, limit, flags };
    return mldt(func, &u, sizeof u);
}

static A_UNUSED u64 ldt_buf[LDT_ENTRIES + 1];
/* Descriptor `entry` as func 0 reads it. */
static A_UNUSED u64 ldt_get(u32 entry) {
    ldt_buf[entry] = 0x5a5a5a5a5a5a5a5aUL;
    mldt(0, ldt_buf, (u64)(entry + 1) * LDT_ENTRY_SIZE);
    return ldt_buf[entry];
}
int test_main(void) {
    u8 *m = (u8 *)syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, m + 2 * A_PAGE, A_PAGE);
    u8 *end = m + 2 * A_PAGE; /* first unmapped byte */

    expect("write_null", mldt(0x11, 0, 16), E32(EFAULT));
    expect("write_null_func1", mldt(1, 0, 16), E32(EFAULT));
    expect("write_low_address", mldt(0x11, (void *)0x1000, 16), E32(EFAULT));
    expect("write_noncanonical", mldt(0x11, (void *)0x8000000000000000UL, 16), E32(EFAULT));
    expect("write_kernel_address", mldt(0x11, (void *)0xffffffff81000000UL, 16), E32(EFAULT));
    struct user_desc *u = (struct user_desc *)(end - 8);
    u->entry_number = 2;
    u->base_addr = 0x1000;
    expect("write_struct_crosses_into_hole", mldt(0x11, u, 16), E32(EFAULT));
    /* Nothing was created: there is still no LDT. */
    expect("no_ldt_after_fault", mldt(0, m, 64), 0);

    struct user_desc *ok = (struct user_desc *)(end - 16);
    ok->entry_number = 2;
    ok->base_addr = 0x1000;
    ok->limit = 0xfff;
    ok->flags = UD_SEG32;
    syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_READ);
    expect("write_read_only_struct", mldt(0x11, ok, 16), 0);
    expect("written", ldt_get(2), 0x0040f30010000fffL);
    syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_NONE);
    expect("write_prot_none_struct", mldt(1, ok, 16), E32(EFAULT));

    /* Reads, now that an LDT exists. */
    syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_READ);
    expect("read0_null", mldt(0, 0, 8), E32(EFAULT));
    expect("read0_null_count_0", mldt(0, 0, 0), 0);
    expect("read0_read_only", mldt(0, m + A_PAGE, 64), E32(EFAULT));
    expect("read0_runs_into_read_only", mldt(0, m + A_PAGE - 8, 64), E32(EFAULT));
    expect("read0_runs_into_hole", mldt(0, end - 8, 64), E32(EFAULT));
    expect("read0_huge_count_into_hole", mldt(0, m, ~0UL), E32(EFAULT));
    expect("read0_fits_exactly", mldt(0, m + A_PAGE - 24, 24), 24);
    expect("read0_fits_entry_2", ((u64 *)(m + A_PAGE - 24))[2], 0x0040f30010000fffL);
    expect("read2_read_only", mldt(2, m + A_PAGE, 16), E32(EFAULT));
    expect("read2_into_read_only", mldt(2, m + A_PAGE - 64, 1000), E32(EFAULT));
    expect("read2_fits_clamped", mldt(2, m + A_PAGE - 128, 1000), 128);
    expect("read2_noncanonical", mldt(2, (void *)0x8000000000000000UL, 8), E32(EFAULT));
    expect("entry_2_kept", ldt_get(2), 0x0040f30010000fffL);
    return finish();
}
