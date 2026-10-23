/* Permission to send with tkill: without CAP_KILL the sender's real or
 * effective uid must equal the target's real or saved uid (the target's
 * effective uid and the sender's saved uid do not count); otherwise EPERM,
 * also for signal 0 and for signals the target blocks. The signal number is
 * checked before permission (EINVAL wins over EPERM) and a missing thread
 * is ESRCH. SIGCONT is allowed within the same session. Root (CAP_KILL)
 * may signal anyone, but not once it has dropped its capabilities.
 * poly-diff: needs=root (setresuid to other uids)
 * poly-diff: syscalls=off (several processes)
 */
#include "poly_ipc.h"

/* linux/capability.h */
#define LINUX_CAPABILITY_VERSION_3 0x20080522
struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

#define NONE 0x7fff /* placeholder for "not run" */

static volatile i64 *res;
static volatile unsigned *ready;

/* Runs in a child as (r, e, s) and stores tkill(target, sig) at res[slot]. */
static void send_as(int slot, unsigned r, unsigned e, unsigned s, int new_session, i64 target, int sig) {
    i64 c = fork();
    if (c == 0) {
        if (new_session && syscall0(SYS_setsid) < 0) syscall1(SYS_exit_group, 2);
        if (syscall3(SYS_setresuid, r, e, s) != 0) syscall1(SYS_exit_group, 1);
        res[slot] = syscall2(SYS_tkill, target, sig);
        syscall1(SYS_exit_group, 0);
    }
    if (c < 0 || wait_exit(c) != 0) res[slot] = NONE;
}

int test_main(void) {
    i64 *page = shared_page(4096);
    check("shared_page", page != 0, 0);
    if (!page) return finish();
    res = page;
    ready = (volatile unsigned *)(page + 100);
    for (int i = 0; i < 32; i++) res[i] = NONE;

    /* The target: real 2000, effective 2001, saved 2002; blocks everything. */
    i64 t = fork();
    if (t == 0) {
        u64 all = ~0UL;
        sig_mask(SIG_BLOCK, all, 0);
        res[31] = syscall3(SYS_setresuid, 2000, 2001, 2002);
        gate_open(&ready[0]);
        for (;;) futex(&ready[1], FUTEX_WAIT, 0, 0, 0, 0);
    }
    check("fork_target", t > 0, t);
    if (t <= 0) return finish();
    gate_wait(&ready[0]);
    check("target_setresuid", res[31] == 0, res[31]);

    send_as(0, 3000, 3000, 3000, 0, t, 0);
    check("unrelated_uid_signal_zero_eperm", res[0] == -EPERM, res[0]);
    send_as(1, 3000, 3000, 3000, 0, t, SIGUSR1);
    check("unrelated_uid_blocked_signal_eperm", res[1] == -EPERM, res[1]);
    send_as(2, 3000, 3000, 3000, 0, t, 65);
    check("bad_signal_before_permission", res[2] == -EINVAL, res[2]);
    send_as(3, 3000, 3000, 3000, 0, 0x7fffffff, 0);
    check("missing_thread_esrch", res[3] == -ESRCH, res[3]);
    send_as(4, 3000, 3000, 3000, 0, 0, 0);
    check("tid_zero_einval", res[4] == -EINVAL, res[4]);

    send_as(5, 2000, 3000, 3000, 0, t, 0);
    check("real_matches_real", res[5] == 0, res[5]);
    send_as(6, 2002, 3000, 3000, 0, t, 0);
    check("real_matches_saved", res[6] == 0, res[6]);
    send_as(7, 3000, 2000, 3000, 0, t, 0);
    check("effective_matches_real", res[7] == 0, res[7]);
    send_as(8, 3000, 2002, 3000, 0, t, SIGUSR1);
    check("effective_matches_saved", res[8] == 0, res[8]);
    send_as(9, 3000, 3000, 2000, 0, t, 0);
    check("sender_saved_does_not_count", res[9] == -EPERM, res[9]);
    send_as(10, 2001, 3000, 3000, 0, t, 0);
    check("target_effective_does_not_count", res[10] == -EPERM, res[10]);
    send_as(11, 3000, 2001, 3000, 0, t, 0);
    check("effective_vs_effective_eperm", res[11] == -EPERM, res[11]);

    send_as(12, 3000, 3000, 3000, 0, t, SIGCONT);
    check("sigcont_same_session", res[12] == 0, res[12]);
    send_as(13, 3000, 3000, 3000, 1, t, SIGCONT);
    check("sigcont_other_session_eperm", res[13] == -EPERM, res[13]);
    send_as(14, 3000, 3000, 3000, 0, t, SIGTERM);
    check("sigterm_same_session_eperm", res[14] == -EPERM, res[14]);

    i64 r = syscall2(SYS_tkill, t, SIGUSR2);
    check("root_may_signal_anyone", r == 0, r);

    /* Without CAP_KILL root (uid 0) is an ordinary uid too. */
    i64 c = fork();
    if (c == 0) {
        struct cap_header h = { LINUX_CAPABILITY_VERSION_3, 0 };
        struct cap_data d[2];
        memset(d, 0, sizeof d);
        if (syscall2(SYS_capset, &h, d) != 0) syscall1(SYS_exit_group, 1);
        if (syscall2(SYS_capget, &h, d) != 0 || d[0].effective != 0) syscall1(SYS_exit_group, 2);
        res[15] = syscall2(SYS_tkill, t, 0);
        res[16] = syscall2(SYS_tkill, gettid(), 0);
        syscall1(SYS_exit_group, 0);
    }
    check("capless_child", c > 0 && wait_exit(c) == 0, c);
    check("no_cap_kill_eperm", res[15] == -EPERM, res[15]);
    check("no_cap_kill_self_ok", res[16] == 0, res[16]);

    syscall2(SYS_kill, t, SIGKILL);
    int st = wait_status(t);
    check("target_killed", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGKILL, st);
    return finish();
}
