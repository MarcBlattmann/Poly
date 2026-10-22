/* Once a write has created the LDT it stays for the life of the address
 * space: after every entry is cleared again func 0 still returns the
 * (clamped) count, all zero bytes. Func 2 never shows the process's
 * table: it reads at most 128 zero bytes whether an LDT exists or not.
 * Even a write that only clears an entry creates the table.
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
static unsigned char b[1024];

int test_main(void) {
    memset(b, 0xa5, sizeof b);
    expect("no_ldt_yet", mldt(0, b, 64), 0);
    expect("clear_only_write", ldt_set(0x11, 6, 0, 0, UD_RO | UD_NP), 0);
    expect("table_exists_after_clear_only", mldt(0, b, 64), 64);
    int zero = 1;
    for (int i = 0; i < 64; i++) zero &= b[i] == 0;
    check("all_zero", zero, 0);

    expect("write_entry_1", ldt_set(0x11, 1, 0x1000, 0xfff, UD_SEG32), 0);
    memset(b, 0xa5, sizeof b);
    expect("read2_with_ldt", mldt(2, b, sizeof b), 128);
    zero = 1;
    for (int i = 0; i < 128; i++) zero &= b[i] == 0;
    check("read2_zero_not_table", zero && b[128] == 0xa5, b[8]);

    expect("clear_entry_1", ldt_set(1, 1, 0, 0, 0), 0);
    memset(b, 0xa5, sizeof b);
    expect("read0_after_clear", mldt(0, b, 100), 100);
    zero = 1;
    for (int i = 0; i < 100; i++) zero &= b[i] == 0;
    check("read0_after_clear_zero", zero && b[100] == 0xa5, b[100]);
    expect("read0_null_count_0", mldt(0, 0, 0), 0);
    return finish();
}
