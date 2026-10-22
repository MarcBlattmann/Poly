/* Clearing entries. A user_desc whose read_exec_only and
 * seg_not_present are 1 and everything else 0 ("empty", lm and the bits
 * above it do not matter) clears the entry to 8 zero bytes with either
 * func. With func 1 (the old mode) base 0 and limit 0 also clear the
 * entry whatever the other flags (but a conforming code segment is
 * still EINVAL with func 1); with func 0x11 they make an ordinary
 * descriptor with base and limit 0. An all-zero user_desc is not empty:
 * it makes a present 16-bit data segment.
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
static void fill(void) { ldt_set(0x11, 3, 0x1000, 0xfff, UD_SEG32); }

int test_main(void) {
    fill();
    expect("filled", ldt_get(3), 0x0040f30010000fffL);
    expect("empty_func_0x11", ldt_set(0x11, 3, 0, 0, UD_RO | UD_NP), 0);
    expect("empty_func_0x11_cleared", ldt_get(3), 0);
    fill();
    expect("empty_func_1", ldt_set(1, 3, 0, 0, UD_RO | UD_NP), 0);
    expect("empty_func_1_cleared", ldt_get(3), 0);
    fill();
    expect("empty_with_lm", ldt_set(0x11, 3, 0, 0, UD_RO | UD_NP | UD_LM), 0);
    expect("empty_with_lm_cleared", ldt_get(3), 0);
    fill();
    expect("old_mode_zero_base_limit", ldt_set(1, 3, 0, 0, UD_SEG32 | UD_CONTENTS(2) | UD_PAGES | UD_USEABLE), 0);
    expect("old_mode_zero_base_limit_cleared", ldt_get(3), 0);
    /* The conforming-code check comes first: func 1 rejects it even when
     * base and limit are 0. */
    fill();
    expect("old_mode_conforming_zero_base_limit", ldt_set(1, 3, 0, 0, UD_CONTENTS(3)), E32(EINVAL));
    expect("old_mode_conforming_not_cleared", ldt_get(3), 0x0040f30010000fffL);
    expect("new_mode_conforming_present_zero", ldt_set(0x11, 3, 0, 0, UD_CONTENTS(3)), E32(EINVAL));
    expect("new_mode_conforming_not_cleared", ldt_get(3), 0x0040f30010000fffL);
    fill();
    expect("new_mode_zero_base_limit", ldt_set(0x11, 3, 0, 0, UD_SEG32), 0);
    expect("new_mode_zero_base_limit_kept", ldt_get(3), 0x0040f30000000000L);
    expect("new_mode_all_zero", ldt_set(0x11, 3, 0, 0, 0), 0);
    expect("new_mode_all_zero_descriptor", ldt_get(3), 0x0000f30000000000L);
    /* Almost empty is not empty. */
    expect("not_empty_base", ldt_set(0x11, 3, 1, 0, UD_RO | UD_NP), 0);
    expect("not_empty_base_descriptor", ldt_get(3), 0x0000710000010000L);
    expect("not_empty_useable", ldt_set(0x11, 3, 0, 0, UD_RO | UD_NP | UD_USEABLE), 0);
    expect("not_empty_useable_descriptor", ldt_get(3), 0x0010710000000000L);
    expect("not_empty_seg32", ldt_set(0x11, 3, 0, 0, UD_RO | UD_NP | UD_SEG32), 0);
    expect("not_empty_seg32_descriptor", ldt_get(3), 0x0040710000000000L);
    expect("not_empty_contents", ldt_set(0x11, 3, 0, 0, UD_RO | UD_NP | UD_CONTENTS(3)), 0);
    expect("not_empty_contents_descriptor", ldt_get(3), 0x00007d0000000000L);
    /* Clearing an entry that was never written works too. */
    expect("clear_unused", ldt_set(0x11, 4000, 0, 0, UD_RO | UD_NP), 0);
    expect("clear_unused_zero", ldt_get(4000), 0);
    return finish();
}
