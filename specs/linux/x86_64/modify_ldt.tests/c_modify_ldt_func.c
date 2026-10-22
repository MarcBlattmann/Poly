/* The function number and the result width. func is a 32-bit int: only
 * 0, 1, 2 and 0x11 exist, anything else is ENOSYS (checked before the
 * pointer is looked at), and the upper half of the register is ignored.
 * The result is a 32-bit int too: an error comes back in %rax as the
 * zero-extended 32-bit value (e.g. 0x00000000ffffffea for EINVAL), not
 * as a sign-extended -22.
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
static unsigned char b[256];

int test_main(void) {
    static const i64 bad[] = { 3, 4, 0x10, 0x12, 0x21, 0x101, 0x7fffffff, -1, -2, (i64)0x80000000L };
    int all = 1;
    i64 got = 0;
    for (unsigned i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        i64 r = mldt(bad[i], b, sizeof b);
        if (r != E32(ENOSYS)) all = 0, got = r;
    }
    check("unknown_func_enosys", all, got);
    expect("enosys_raw_value", mldt(3, b, 16), 0xffffffdaL);
    expect("enosys_before_pointer", mldt(5, 0, 16), E32(ENOSYS));
    check("error_not_sign_extended", mldt(3, b, 16) > 0, mldt(3, b, 16));
    expect("einval_raw_value", mldt(1, b, 15), 0xffffffeaL);
    expect("efault_raw_value", mldt(1, 0, 16), 0xfffffff2L);

    /* Only the low 32 bits of func count. */
    expect("func_high_bits_read0", mldt(1L << 32, b, 64), 0);
    expect("func_high_bits_read2", mldt((1L << 32) | 2, b, 500), 128);
    expect("func_high_bits_unknown", mldt((7L << 32) | 3, b, 16), E32(ENOSYS));
    expect("func_high_bits_write_size", mldt((1L << 40) | 1, b, 15), E32(EINVAL));
    struct user_desc u = { 1, 0x1000, 0xfff, UD_SEG32 };
    expect("func_high_bits_write", mldt((0xffL << 32) | 0x11, &u, sizeof u), 0);
    expect("func_high_bits_write_took", ldt_get(1), 0x0040f30010000fffL);
    return finish();
}
