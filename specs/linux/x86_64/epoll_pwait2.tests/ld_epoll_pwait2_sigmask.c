/* epoll_pwait2's sigmask replaces the signal mask for the duration of the
 * call. SIGUSR1 blocked and pending: a call whose sigmask unblocks it and
 * that would sleep (NULL or a non-zero timeout) runs the handler and
 * returns EINTR (4), and the caller's mask (SIGUSR1 blocked) is back
 * afterwards; with a zero timeout the call only polls, returns 0 and the
 * signal stays pending; a call without sigmask, or with one that keeps it
 * blocked, leaves it pending and returns normally. A signal that arrives during the wait
 * while sigmask blocks it does not end the wait (the call times out with
 * 0); it is delivered when the old mask returns.
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_epoll_ctl 233
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_epoll_pwait2 441

struct epoll_event {
    u32 events;
    u64 data;
} __attribute__((packed));

void ld_restorer(void);
__asm__(".text\n"
        ".global ld_restorer\n"
        "ld_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static struct epoll_event evs[4];
static volatile i32 usr1, alrm;
static void on_usr1(int s) {
    (void)s;
    usr1++;
}
static void on_alrm(int s) {
    (void)s;
    alrm++;
}

static u64 cur_mask(void) {
    u64 m = 0;
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, 0, &m, 8);
    return m;
}

static u64 pending(void) {
    u64 p = 0;
    syscall2(SYS_rt_sigpending, &p, 8);
    return p;
}

int test_main(void) {
    struct a_sigaction sa = {(u64)on_usr1, SA_RESTORER, (u64)ld_restorer, 0};
    syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8);
    sa.handler = (u64)on_alrm;
    syscall4(SYS_rt_sigaction, SIGALRM, &sa, 0, 8);
    i64 ep = syscall1(SYS_epoll_create1, 0);
    struct a_timespec zero = {0, 0};

    u64 blk = A_SIGBIT(SIGUSR1);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &blk, 0, 8);
    syscall2(SYS_kill, syscall0(SYS_getpid), SIGUSR1);
    expect("pending_blocked", pending() & blk, blk);

    expect("no_sigmask_stays_pending", SC6(SYS_epoll_pwait2, ep, evs, 4, &zero, 0, 8), 0);
    u64 keep = A_SIGBIT(SIGUSR1) | A_SIGBIT(SIGUSR2);
    expect("blocking_sigmask_stays_pending", SC6(SYS_epoll_pwait2, ep, evs, 4, &zero, &keep, 8), 0);
    expect("handler_not_run", usr1, 0);

    u64 empty = 0;
    expect("zero_timeout_only_polls", SC6(SYS_epoll_pwait2, ep, evs, 4, &zero, &empty, 8), 0);
    expect("zero_timeout_handler_not_run", usr1, 0);
    expect("zero_timeout_still_pending", pending() & blk, blk);
    expect("zero_timeout_mask_restored", cur_mask(), blk);

    expect("unlimited_wait_eintr", SC6(SYS_epoll_pwait2, ep, evs, 4, 0, &empty, 8), -EINTR);
    expect("handler_ran", usr1, 1);
    expect("mask_restored", cur_mask(), blk);
    expect("nothing_pending", pending(), 0);

    syscall2(SYS_kill, syscall0(SYS_getpid), SIGUSR1);
    struct a_timespec t5s = {5, 0};
    expect("timed_wait_eintr", SC6(SYS_epoll_pwait2, ep, evs, 4, &t5s, &empty, 8), -EINTR);
    expect("handler_ran_again", usr1, 2);
    expect("mask_restored_again", cur_mask(), blk);

    /* An event ready and the signal pending: the event wins, the signal
     * stays pending (the old mask returns without delivering it). */
    i64 efd = syscall2(SYS_eventfd2, 1, 0);
    struct epoll_event e = {1 /* EPOLLIN */, 9};
    SC6(SYS_epoll_ctl, ep, 1 /* EPOLL_CTL_ADD */, efd, &e, 0, 0);
    syscall2(SYS_kill, syscall0(SYS_getpid), SIGUSR1);
    expect("event_beats_signal", SC6(SYS_epoll_pwait2, ep, evs, 4, 0, &empty, 8), 1);
    expect("event_handler_not_run", usr1, 2);
    expect("event_signal_still_pending", pending() & blk, blk);
    SC6(SYS_epoll_ctl, ep, 2 /* EPOLL_CTL_DEL */, efd, 0, 0, 0);
    a_close(efd);
    expect("drain_with_handler", SC6(SYS_epoll_pwait2, ep, evs, 4, 0, &empty, 8), -EINTR);
    expect("handler_ran_third", usr1, 3);

    /* SIGALRM during the wait, blocked by sigmask: the wait times out. */
    struct a_itimerval it = {{0, 0}, {0, 20000}};
    u64 block_alrm = A_SIGBIT(SIGALRM);
    struct a_timespec t100 = {0, 100000000};
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
    expect("blocked_alarm_times_out", SC6(SYS_epoll_pwait2, ep, evs, 4, &t100, &block_alrm, 8), 0);
    expect("alarm_delivered_after_return", alrm, 1);
    expect("mask_after_alarm", cur_mask(), blk);
    a_close(ep);
    return finish();
}
