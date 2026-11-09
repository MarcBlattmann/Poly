/* Filters form a chain per task: a child starts with a copy of its
 * parent's chain and adds to its own copy only. Process A installs F_A
 * (sched_yield -> ERRNO 5); its child B adds F_B (getuid -> ERRNO 6);
 * B's child C adds F_C (getgid -> KILL_PROCESS). So A sees only F_A, B
 * sees F_A and F_B, and C sees all three; C dies by SIGSYS when it calls
 * getgid and neither B nor A is affected; a second child of A (a sibling
 * of B) sees only F_A. The count of filters is visible only through these
 * effects, not printed. no_new_privs, once set, is inherited by all and
 * cannot be cleared; the seccomp mode is 2 everywhere below A's install.
 * poly-diff: syscalls=off (several processes)
 */
#include "p_seccomp.h"

static void install(struct pk_insn *f, u16 n) {
    struct pk_prog p = {n, f};
    if (syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &p) != 0) syscall1(SYS_exit_group, 99);
}

static i32 child_c(void) {
    i32 st = -1;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        static struct pk_insn fc[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getgid, 0, 1), PK_RET(SECCOMP_RET_KILL_PROCESS), PK_ALLOW};
        install(fc, 4);
        int bad = 0;
        if (syscall0(SYS_sched_yield) != -5) bad |= 1;
        if (syscall0(SYS_getuid) != -6) bad |= 2;
        if (syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0) != 2) bad |= 4;
        if (syscall5(SYS_prctl, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 1) bad |= 8;
        if (bad) syscall1(SYS_exit_group, bad);
        syscall0(SYS_getgid); /* dies here */
        syscall1(SYS_exit_group, 77);
    }
    syscall4(SYS_wait4, pid, &st, 0, 0);
    return st;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        /* process A */
        volatile i32 *s = a_shared();
        static struct pk_insn fa[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_sched_yield, 0, 1), PK_ERR(5), PK_ALLOW};
        static struct pk_insn fb[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getuid, 0, 1), PK_ERR(6), PK_ALLOW};
        syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        install(fa, 4);
        s[0] = syscall0(SYS_sched_yield);     /* -5 */
        s[1] = (i32)(syscall0(SYS_getuid) >= 0); /* 1: F_B is not in A's chain */
        /* child B */
        i32 stb = -1;
        i64 b = a_fork_sync();
        if (b == 0) {
            install(fb, 4);
            s[2] = syscall0(SYS_sched_yield);   /* -5 */
            s[3] = syscall0(SYS_getuid);        /* -6 */
            s[4] = (i32)(syscall0(SYS_getgid) >= 0); /* 1: F_C is not in B's chain */
            s[5] = child_c();                   /* wait status of C */
            s[6] = (i32)(syscall0(SYS_getgid) >= 0); /* B survived C's death */
            syscall1(SYS_exit_group, 0);
        }
        syscall4(SYS_wait4, b, &stb, 0, 0);
        s[7] = stb;
        /* a sibling of B, made after B ended */
        i64 d = a_fork_sync();
        if (d == 0) {
            s[8] = (i32)(syscall0(SYS_getuid) >= 0); /* 1: no F_B here */
            s[9] = syscall0(SYS_sched_yield);       /* -5 */
            syscall1(SYS_exit_group, 0);
        }
        i32 std = -1;
        syscall4(SYS_wait4, d, &std, 0, 0);
        s[10] = std;
        s[11] = syscall0(SYS_sched_yield); /* A still -5 */
        syscall1(SYS_exit_group, 0);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    volatile i32 *s = a_shared();
    expect("A_ended_normally", st, 0);
    expect("A_sees_F_A", s[0], -5);
    expect("A_does_not_see_F_B", s[1], 1);
    expect("B_sees_F_A", s[2], -5);
    expect("B_sees_F_B", s[3], -6);
    expect("B_does_not_see_F_C", s[4], 1);
    check("C_died_of_sigsys", A_WIFSIGNALED(s[5]) && A_WTERMSIG(s[5]) == SIGSYS, s[5]);
    expect("B_survived_C", s[6], 1);
    expect("B_ended_normally", s[7], 0);
    expect("sibling_has_no_F_B", s[8], 1);
    expect("sibling_sees_F_A", s[9], -5);
    expect("sibling_ended_normally", s[10], 0);
    expect("A_unchanged_by_the_others", s[11], -5);
    expect("test_process_unfiltered", syscall0(SYS_sched_yield), 0);
    return finish();
}
