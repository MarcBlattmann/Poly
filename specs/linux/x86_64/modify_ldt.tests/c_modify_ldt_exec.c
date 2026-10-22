/* execve gives the process a new address space without an LDT: after
 * it, func 0 returns 0 again and the old selectors are invalid.
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
static int lar_ok(u64 sel) {
    u64 v = 0;
    u8 z;
    __asm__ volatile("lar %2, %0\n\tsetz %1" : "=r"(v), "=q"(z) : "r"(sel) : "cc");
    return z;
}

static int phase2(void) {
    passed = (int)a_atou(a_argv()[2]);
    u64 tmp[8];
    expect("after_exec_no_ldt", mldt(0, tmp, sizeof tmp), 0);
    check("after_exec_selector_invalid", !lar_ok(5 << 3 | 7), 0);
    expect("after_exec_write_again", ldt_set(0x11, 5, 0x7000, 0x77, UD_SEG32), 0);
    expect("after_exec_read", ldt_get(5), 0x0040f30070000077L);
    return finish();
}

int test_main(void) {
    if (a_argc() > 2 && str_eq(a_argv()[1], "2")) return phase2();
    expect("write_entry_5", ldt_set(0x11, 5, 0x5000, 0x55, UD_SEG32), 0);
    check("selector_valid", lar_ok(5 << 3 | 7), 0);
    char num[24];
    a_utoa(num, (u64)passed);
    char *argv[] = { "m", "2", num, 0 };
    char *envp[] = { 0 };
    i64 r = syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve", r, 0);
    return finish();
}
