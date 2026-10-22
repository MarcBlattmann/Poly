/* Writing one entry creates the LDT; func 0 then reads it as an array
 * of 8-byte descriptors indexed by entry number. Once an LDT exists,
 * func 0 returns min(count, 65536) (8192 entries of 8 bytes): entries
 * that were never written, and anything past the table, read as zero
 * bytes; a count that is not a multiple of 8 copies a partial
 * descriptor; nothing beyond the returned count is written.
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
static unsigned char b[70000];

int test_main(void) {
    expect("write_entry_5", ldt_set(1, 5, 0x1000, 0xfff, UD_SEG32), 0);
    memset(b, 0xa5, sizeof b);
    expect("read_100", mldt(0, b, 100), 100);
    u64 *d = (u64 *)b;
    check("entries_0_to_4_zero", !d[0] && !d[1] && !d[2] && !d[3] && !d[4], d[0]);
    expect("entry_5_descriptor", d[5], 0x0040f30010000fffL);
    int zero = 1;
    for (int i = 48; i < 100; i++) zero &= b[i] == 0;
    check("past_table_reads_zero", zero, b[48]);
    check("nothing_past_count", b[100] == 0xa5, b[100]);

    memset(b, 0xa5, sizeof b);
    expect("read_45", mldt(0, b, 45), 45);
    check("partial_descriptor",
          b[40] == 0xff && b[41] == 0x0f && b[42] == 0 && b[43] == 0x10 && b[44] == 0 && b[45] == 0xa5, b[43]);

    expect("read_0", mldt(0, b, 0), 0);
    memset(b, 0xa5, sizeof b);
    expect("read_65536", mldt(0, b, 65536), 65536);
    check("read_65536_last_zero", b[65535] == 0 && b[65536] == 0xa5, b[65535]);
    memset(b, 0xa5, sizeof b);
    expect("read_clamped_to_65536", mldt(0, b, 69999), 65536);
    check("clamped_writes_nothing_more", b[65536] == 0xa5, b[65536]);
    expect("read_count_huge", mldt(0, b, ~0UL), 65536);
    expect("entry_5_still", ((u64 *)b)[5], 0x0040f30010000fffL);

    /* A second entry below the first leaves the first alone. */
    expect("write_entry_2", ldt_set(0x11, 2, 0x2000, 0x10, UD_SEG32 | UD_RO), 0);
    expect("entry_2", ldt_get(2), 0x0040f10020000010L);
    expect("entry_5_kept", ldt_get(5), 0x0040f30010000fffL);
    /* Rewriting an entry replaces it. */
    expect("rewrite_entry_5", ldt_set(1, 5, 0xabcd0000, 0x3, UD_SEG32 | UD_PAGES), 0);
    expect("entry_5_replaced", ldt_get(5), 0xabc0f3cd00000003L);
    return finish();
}
