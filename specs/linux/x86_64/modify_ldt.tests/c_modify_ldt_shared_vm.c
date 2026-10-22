/* Processes sharing the address space (CLONE_VM) share one LDT: an
 * entry written by the clone is seen by the creator, and the creator's
 * first write creates the table for both.
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
static volatile i32 done;
static volatile i64 child_read0, child_set;
static volatile u64 child_entry;

static int child(void *arg) {
    (void)arg;
    u64 tmp[4];
    child_read0 = mldt(0, tmp, sizeof tmp);
    child_entry = ldt_get(1);
    child_set = ldt_set(0x11, 2, 0x2000, 0x22, UD_SEG32);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    expect("write_entry_1", ldt_set(0x11, 1, 0x1000, 0x11, UD_SEG32), 0);
    i64 pid = a_spawn(0, &done, child, 0);
    check("spawn", pid > 0, pid);
    a_spin_until((volatile i32 *)&done, 0, 1UL << 40);
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_read0_full", child_read0, 32);
    expect("child_sees_entry_1", child_entry, 0x0040f30010000011L);
    expect("child_write", child_set, 0);
    expect("parent_sees_entry_2", ldt_get(2), 0x0040f30020000022L);
    return finish();
}
