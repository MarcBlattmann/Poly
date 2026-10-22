/* EINVAL from the write functions (1 and 0x11): count other than 16
 * (sizeof struct user_desc, all 64 bits compared), entry_number 8192 or
 * more, a conforming code segment (contents 3) with func 1 or present.
 * The count is checked before the pointer (NULL with a bad count is
 * EINVAL), the pointer before the contents (NULL with count 16 is
 * EFAULT). A rejected write changes nothing.
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
    expect("setup", ldt_set(0x11, 7, 0x1000, 0xfff, UD_SEG32), 0);
    struct user_desc u = { 7, 0x2000, 0x1, UD_SEG32 };
    static const u64 counts[] = { 0, 1, 8, 15, 17, 24, 32, 4096, 16 | (1UL << 32), ~0UL };
    int all = 1;
    i64 got = 0;
    for (int f = 0; f < 2; f++)
        for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {
            i64 r = mldt(f ? 0x11 : 1, &u, counts[i]);
            if (r != E32(EINVAL)) all = 0, got = (i64)counts[i];
        }
    check("count_not_16", all, got);
    expect("bad_count_null", mldt(1, 0, 15), E32(EINVAL));
    expect("bad_count_null_0x11", mldt(0x11, 0, 0), E32(EINVAL));

    expect("entry_8192", ldt_set(0x11, 8192, 0x2000, 1, UD_SEG32), E32(EINVAL));
    expect("entry_8193_func1", ldt_set(1, 8193, 0x2000, 1, UD_SEG32), E32(EINVAL));
    expect("entry_max", ldt_set(0x11, 0xffffffff, 0x2000, 1, UD_SEG32), E32(EINVAL));
    expect("entry_high_bit", ldt_set(0x11, 0x80000007, 0x2000, 1, UD_SEG32), E32(EINVAL));
    expect("entry_big_even_if_empty", ldt_set(0x11, 9000, 0, 0, UD_RO | UD_NP), E32(EINVAL));

    expect("conforming_present", ldt_set(0x11, 7, 0, 7, UD_SEG32 | UD_CONTENTS(3)), E32(EINVAL));
    expect("conforming_func1", ldt_set(1, 7, 0, 7, UD_SEG32 | UD_CONTENTS(3) | UD_NP), E32(EINVAL));
    expect("conforming_func1_present", ldt_set(1, 7, 0x10, 7, UD_CONTENTS(3)), E32(EINVAL));
    expect("conforming_np_ok", ldt_set(0x11, 6, 0, 7, UD_SEG32 | UD_CONTENTS(3) | UD_NP), 0);

    expect("unchanged_after_errors", ldt_get(7), 0x0040f30010000fffL);
    return finish();
}
