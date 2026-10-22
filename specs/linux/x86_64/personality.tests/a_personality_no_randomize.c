/* personality: ADDR_NO_RANDOMIZE (0x0040000) makes the address-space
 * layout of a program started by execve repeatable: two runs of the same
 * program with the same arguments and environment get the same initial
 * stack pointer, AT_RANDOM address and program break. This program runs
 * itself twice with the flag; each new image reports through a pipe.
 * poly-diff: syscalls=off (child processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
#define QUERY 0xffffffffUL
/* linux/personality.h */
#define ADDR_NO_RANDOMIZE 0x0040000
/* linux/auxvec.h */
#define AT_RANDOM 25

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

static int run(int wfd) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        personality(ADDR_NO_RANDOMIZE);
        char *argv[] = {"x", "report", 0};
        char *envp[] = {0};
        if (wfd != 9) syscall2(SYS_dup2, wfd, 9);
        syscall3(SYS_execve, a_self_path(), argv, envp);
        syscall1(SYS_exit_group, 99);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    return st;
}

int test_main(void) {
    if (a_argc() > 1 && str_eq(a_argv()[1], "report")) {
        u64 rep[4] = {(u64)poly_initial_sp, a_auxv(AT_RANDOM), (u64)syscall1(SYS_brk, 0),
                      (u64)personality(QUERY)};
        a_write(9, rep, sizeof rep);
        return 0;
    }
    i64 initial = personality(QUERY);
    a_block_sigchld();
    i32 p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    u64 a[4], b[4];
    expect("first_run", run(p[1]), 0);
    expect("first_report", a_read(p[0], a, sizeof a), sizeof a);
    expect("second_run", run(p[1]), 0);
    expect("second_report", a_read(p[0], b, sizeof b), sizeof b);
    expect("flag_kept_over_exec", (i64)a[3], ADDR_NO_RANDOMIZE);
    check("same_stack", a[0] == b[0], 0);
    check("same_at_random", a[1] == b[1], 0);
    check("same_brk", a[2] == b[2], 0);
    expect("parent_unchanged", personality(QUERY), initial);
    return finish();
}
