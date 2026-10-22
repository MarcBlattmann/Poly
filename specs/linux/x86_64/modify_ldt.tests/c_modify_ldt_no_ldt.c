/* A new process has no LDT of its own: func 0 reads nothing and returns
 * 0 whatever the buffer and count (even NULL), leaving the buffer as it
 * was. Func 2 (the "default LDT") returns min(count, 128) zero bytes
 * and needs a writable buffer for them.
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
static unsigned char b[512];

int test_main(void) {
    memset(b, 0xa5, sizeof b);
    expect("read0_count_100", mldt(0, b, 100), 0);
    check("read0_buffer_untouched", b[0] == 0xa5 && b[99] == 0xa5, b[0]);
    expect("read0_count_0", mldt(0, b, 0), 0);
    expect("read0_count_65536", mldt(0, b, 65536), 0);
    expect("read0_count_huge", mldt(0, b, ~0UL), 0);
    expect("read0_null", mldt(0, 0, 100), 0);
    expect("read0_bad_pointer", mldt(0, (void *)0x10, 4096), 0);

    memset(b, 0xa5, sizeof b);
    expect("read2_count_100", mldt(2, b, 100), 100);
    check("read2_zeroes_count_bytes", b[0] == 0 && b[99] == 0 && b[100] == 0xa5, b[100]);
    memset(b, 0xa5, sizeof b);
    expect("read2_count_500", mldt(2, b, 500), 128);
    int zero = 1;
    for (int i = 0; i < 128; i++) zero &= b[i] == 0;
    check("read2_128_zero_bytes", zero && b[128] == 0xa5, b[128]);
    expect("read2_count_huge", mldt(2, b, ~0UL), 128);
    expect("read2_count_0", mldt(2, b, 0), 0);
    expect("read2_null_count_0", mldt(2, 0, 0), 0);
    expect("read2_null", mldt(2, 0, 8), E32(EFAULT));
    /* Still no LDT afterwards. */
    expect("read0_still_empty", mldt(0, b, 64), 0);
    return finish();
}
