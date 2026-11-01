/* The persona survives execve, except READ_IMPLIES_EXEC (0x0400000),
 * which execve of an x86-64 ELF program with a non-executable stack
 * (PT_GNU_STACK without PF_X, as this one) clears. Other flags
 * (ADDR_NO_RANDOMIZE 0x0040000, UNAME26 0x0020000, PER_LINUX32 0x0008,
 * SHORT_INODE 0x1000000, ADDR_LIMIT_32BIT 0x0800000, ADDR_LIMIT_3GB
 * 0x8000000) and the domain byte are kept. The program runs itself again
 * with the expected value as an argument and checks it there.
 * poly-diff: syscalls=off (parent and exec'd child run at once)
 */
#include "a_sys.h"

#define SYS_personality 135
#define QUERY 0xffffffffUL
#define READ_IMPLIES_EXEC 0x0400000UL

static i64 pers(u64 v) { return syscall1(SYS_personality, v); }

static int run_child(u64 set, u64 want) {
    char want_s[24];
    a_utoa(want_s, want);
    char *argv[] = { (char *)a_self_path(), want_s, 0 };
    char *envp[] = { 0 };
    pers(set);
    i64 c = a_fork_sync();
    if (c == 0) {
        syscall3(SYS_execve, argv[0], argv, envp);
        exit_group(99);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    return A_WIFEXITED(st) ? A_WEXITSTATUS(st) : 100;
}

int test_main(void) {
    if (a_argc() == 2) {
        /* The exec'd program: exit 0 if the persona is the expected one. */
        u64 want = a_atou(a_argv()[1]);
        exit_group(pers(QUERY) == (i64)want ? 0 : 1);
    }
    i64 orig = pers(QUERY);
    check("plain_kept", run_child(0x0000000, 0x0000000) == 0, 0);
    check("domain_kept", run_child(0x0000006, 0x0000006) == 0, 0);
    check("flags_kept", run_child(0x0040000 | 0x0020000 | 0x0008, 0x0040000 | 0x0020000 | 0x0008) == 0, 0);
    check("addr_limits_kept", run_child(0x1000000 | 0x0800000 | 0x8000000, 0x1000000 | 0x0800000 | 0x8000000) == 0, 0);
    check("read_implies_exec_cleared", run_child(READ_IMPLIES_EXEC, 0) == 0, 0);
    check("read_implies_exec_cleared_others_kept",
          run_child(READ_IMPLIES_EXEC | 0x0040000 | 0x0008, 0x0040000 | 0x0008) == 0, 0);
    check("parent_keeps_its_value", pers(QUERY) == (i64)(READ_IMPLIES_EXEC | 0x0040000 | 0x0008), pers(QUERY));
    pers((u64)orig);
    check("restored", pers(QUERY) == orig, 0);
    return finish();
}
