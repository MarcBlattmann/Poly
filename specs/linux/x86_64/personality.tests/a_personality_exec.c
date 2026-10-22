/* personality: execve of an ordinary 64-bit program keeps the persona,
 * except that READ_IMPLIES_EXEC (0x0400000) is cleared. Every other bit
 * and execution domain survives (PER_LINUX32 included). This program
 * runs itself again; the new image reports what it sees.
 * poly-diff: syscalls=off (child processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
#define QUERY 0xffffffffUL
/* linux/personality.h */
#define READ_IMPLIES_EXEC 0x0400000

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

/* Run this program again with persona `p`; return what the new image
 * saw (from its exit status: 0 = expected value seen, else 1), or -1. */
static i64 exec_with(u64 p, u64 want) {
    char wbuf[24];
    a_utoa(wbuf, want);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        personality(p);
        char *argv[] = {"x", "check", wbuf, 0};
        char *envp[] = {0};
        syscall3(SYS_execve, a_self_path(), argv, envp);
        syscall1(SYS_exit_group, 99);
    }
    i32 st = -1;
    if (syscall4(SYS_wait4, pid, &st, 0, 0) != pid || !A_WIFEXITED(st)) return -1;
    return A_WEXITSTATUS(st);
}

int test_main(void) {
    if (a_argc() > 2 && str_eq(a_argv()[1], "check"))
        return (u64)personality(QUERY) == a_atou(a_argv()[2]) ? 0 : 1;
    i64 initial = personality(QUERY);
    a_block_sigchld();
    expect("linux_kept", exec_with(0, 0), 0);
    expect("linux32_kept", exec_with(0x0008, 0x0008), 0);
    expect("bsd_kept", exec_with(0x0006, 0x0006), 0);
    expect("odd_domain_kept", exec_with(0x0042, 0x0042), 0);
    expect("uname26_kept", exec_with(0x0020000, 0x0020000), 0);
    expect("no_randomize_kept", exec_with(0x0040000, 0x0040000), 0);
    expect("mmap_page_zero_kept", exec_with(0x0100000, 0x0100000), 0);
    expect("compat_layout_kept", exec_with(0x0200000, 0x0200000), 0);
    expect("limit_32bit_kept", exec_with(0x0800000, 0x0800000), 0);
    expect("sticky_kept", exec_with(0x4000000, 0x4000000), 0);
    expect("limit_3gb_kept", exec_with(0x8000000, 0x8000000), 0);
    expect("bit31_kept", exec_with(0x80000000UL, 0x80000000UL), 0);
    expect("read_implies_exec_cleared", exec_with(READ_IMPLIES_EXEC, 0), 0);
    expect("rie_cleared_rest_kept", exec_with(READ_IMPLIES_EXEC | 0x0010008, 0x0010008), 0);
    expect("all_bits_but_rie", exec_with(0xfffffffeUL, 0xfffffffeUL & ~(u64)READ_IMPLIES_EXEC), 0);
    expect("parent_unchanged", personality(QUERY), initial);
    return finish();
}
