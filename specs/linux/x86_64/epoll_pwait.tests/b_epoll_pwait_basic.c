/* epoll_pwait: epoll_wait with a signal mask for the duration of the call;
 * a NULL mask is plain epoll_wait (sigsetsize then not checked); a mask
 * that unblocks a pending signal ends the call with EINTR after the
 * handler, and the old mask comes back; a mask that keeps it blocked lets
 * the call return its events; sigsetsize other than 8 is EINVAL. */
#include "poly_ipc.h"

static volatile int hits;
static void on_usr1(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    int e = (int)syscall1(SYS_epoll_create1, 0);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_epoll_event ev = { EPOLLOUT, 4 }, out[2];
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[1], &ev);

    i64 r = syscall6(SYS_epoll_pwait, e, (i64)out, 2, 0, 0, 8);
    check("null_mask_like_wait", r == 1 && out[0].data == 4, r);
    r = syscall6(SYS_epoll_pwait, e, (i64)out, 2, 0, 0, 3);
    check("null_mask_size_ignored", r == 1, r);

    sig_handler(SIGUSR1, on_usr1, 0);
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1), 0);
    raise_sig(SIGUSR1);
    u64 keep = SIGBIT(SIGUSR1);
    r = syscall6(SYS_epoll_pwait, e, (i64)out, 2, 0, (i64)&keep, 8);
    check("blocked_by_mask_returns_events", r == 1 && hits == 0, r);
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_DEL, p[1], 0);
    u64 none = 0;
    r = syscall6(SYS_epoll_pwait, e, (i64)out, 2, -1, (i64)&none, 8);
    check("mask_unblocks_pending_eintr", r == -EINTR && hits == 1, r);
    check("old_mask_back", sig_current_mask() == SIGBIT(SIGUSR1), (i64)sig_current_mask());
    r = syscall6(SYS_epoll_pwait, e, (i64)out, 2, 0, (i64)&none, 4);
    check("sigsetsize_4_einval", r == -EINVAL, r);
    r = syscall6(SYS_epoll_pwait, e, (i64)out, 2, 0, 8, 8);
    check("bad_mask_efault", r == -EFAULT, r);
    r = syscall6(SYS_epoll_pwait, e, (i64)out, 0, 0, (i64)&none, 8);
    check("maxevents_0_einval", r == -EINVAL, r);
    sig_mask(SIG_SETMASK, 0, 0);
    return finish();
}
