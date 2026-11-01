/* tkill permission rules for a sender without CAP_KILL: the sender's real
 * or effective uid must equal the target's real or saved uid; the target's
 * effective uid does not count. Otherwise EPERM, and nothing is sent. The
 * signal number is checked before permission (an invalid signal to a
 * thread the caller may not signal is EINVAL); a missing thread is ESRCH
 * before either. Signal 0 is checked like any other signal. Root (with
 * CAP_KILL) may signal anyone.
 * poly-diff: needs=root (sets uids of other users)
 * poly-diff: syscalls=off (target processes run while being signalled)
 */
#include "poly_ipc.h"

#define CLONE_VFORK 0x4000

static i64 tkill(i64 tid, i64 sig) { return syscall2(SYS_tkill, tid, sig); }

/* A process that takes the given real, effective and saved uids, then
 * waits; it tells the caller (shared page) when it is ready. */
static volatile unsigned *ready;
static volatile int *leftover;
static i64 target(int slot, i64 r, i64 e, i64 s) {
    i64 c = fork();
    if (c == 0) {
        i64 ok = syscall3(SYS_setresuid, r, e, s);
        gate_open(&ready[slot]);
        if (ok != 0) syscall1(SYS_exit, 3); /* not root: the checks fail */
        for (;;) syscall0(SYS_pause);
    }
    gate_wait(&ready[slot]);
    return c;
}

static int body(void) {
    i64 parent = syscall0(SYS_getppid);
    /* Targets made while still root. */
    i64 t_e3000 = target(0, 1000, 3000, 1000); /* only its euid is 3000 */
    i64 t_s3000 = target(1, 1000, 1000, 3000); /* its saved uid is 3000 */
    i64 t_r3000 = target(2, 3000, 1000, 1000); /* its real uid is 3000 */
    i64 t_root = target(3, 0, 0, 0);

    i64 r = syscall3(SYS_setresuid, 3000, 3000, 3000);
    check("dropped_to_uid_3000", r == 0, r);

    check("own_thread_ok", tkill(gettid(), 0) == 0, 0);
    r = tkill(parent, 0);
    check("root_parent_eperm", r == -EPERM, r);
    r = tkill(parent, SIGKILL);
    check("root_parent_sigkill_eperm", r == -EPERM, r);
    r = tkill(parent, 999);
    check("bad_signal_before_permission", r == -EINVAL, r);
    r = tkill(0x7fffffff, 999);
    check("missing_before_signal", r == -ESRCH, r);
    r = tkill(t_root, 0);
    check("root_target_eperm", r == -EPERM, r);
    r = tkill(t_e3000, 0);
    check("target_euid_does_not_count", r == -EPERM, r);
    r = tkill(t_s3000, 0);
    check("target_saved_uid_matches", r == 0, r);
    r = tkill(t_r3000, 0);
    check("target_real_uid_matches", r == 0, r);

    /* Real signals: a permitted one ends the target. */
    r = tkill(t_r3000, SIGKILL);
    check("permitted_sigkill", r == 0, r);
    int s = wait_status(t_r3000);
    check("permitted_target_killed", W_SIGNALED(s) && W_TERMSIG(s) == SIGKILL, s);
    r = tkill(t_s3000, SIGKILL);
    check("permitted_sigkill_saved", r == 0, r);
    s = wait_status(t_s3000);
    check("permitted_saved_killed", W_SIGNALED(s) && W_TERMSIG(s) == SIGKILL, s);
    r = tkill(t_e3000, SIGKILL);
    check("refused_sigkill_eperm", r == -EPERM, r);
    r = tkill(t_e3000, 0);
    check("refused_target_still_there", r == -EPERM, r);
    leftover[0] = (int)t_e3000;
    leftover[1] = (int)t_root;
    return finish();
}

int test_main(void) {
    if (syscall0(SYS_geteuid) != 0) {
        check("runs_as_root", 0, syscall0(SYS_geteuid));
        return finish();
    }
    ready = shared_page(4096);
    leftover = (volatile int *)(ready + 16);
    i64 c = syscall5(SYS_clone, CLONE_VFORK | SIGCHLD, 0, 0, 0, 0);
    if (c == 0) syscall1(SYS_exit_group, body());
    int s = wait_status(c);
    /* The targets the child could not end: root (CAP_KILL) may. */
    int ok = 1;
    for (int i = 0; i < 2; i++)
        if (leftover[i] > 0 && tkill(leftover[i], SIGKILL) != 0) ok = 0;
    if (!ok) put("not ok root_kills_leftovers\n");
    return W_EXITED(s) && ok ? W_EXITCODE(s) : 1;
}
