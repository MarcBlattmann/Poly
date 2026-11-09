/* epoll_pwait2's timeout is a relative struct timespec: {0, 0} only polls
 * (returns 0 at once with nothing ready), {0, 30 ms} waits at least 30 ms
 * and returns 0, 1 ns returns 0, NULL waits without limit. The timespec is
 * not written. An event that is ready, or becomes ready during the wait
 * (a child writes to an eventfd), ends the wait at once with the count.
 * poly-diff: syscalls=off (the child's write races with the wait)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_epoll_ctl 233
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_epoll_pwait2 441
/* linux/eventpoll.h */
#define EPOLL_CTL_ADD 1
#define EPOLLIN 0x001

struct epoll_event {
    u32 events;
    u64 data;
} __attribute__((packed));

static struct epoll_event evs[8];
static volatile i32 child_done;
static i64 efd;

static i64 ns_of(struct a_timespec *t) { return t->tv_sec * 1000000000L + t->tv_nsec; }
static i64 now_ns(void) {
    struct a_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return ns_of(&t);
}

static int writer(void *arg) {
    (void)arg;
    a_spin_ticks(100000000); /* let the parent go to sleep first */
    u64 one = 1;
    a_write(efd, &one, 8);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 ep = syscall1(SYS_epoll_create1, 0);
    efd = syscall2(SYS_eventfd2, 0, 0);
    struct epoll_event e = {EPOLLIN, 7};
    SC6(SYS_epoll_ctl, ep, EPOLL_CTL_ADD, efd, &e, 0, 0);

    struct a_timespec zero = {0, 0}, t30 = {0, 30000000}, ns1 = {0, 1}, big = {100000, 0};
    /* The 25 ms regions hold only the calls under test: their results are
     * kept and checked after the second clock read, so the test's own
     * output does not count against the limit. */
    i64 t0 = now_ns();
    i64 r_zero = SC6(SYS_epoll_pwait2, ep, evs, 8, &zero, 0, 8);
    i64 r_ns1 = SC6(SYS_epoll_pwait2, ep, evs, 8, &ns1, 0, 8);
    i64 el = now_ns() - t0;
    expect("poll_returns_0", r_zero, 0);
    expect("one_ns_returns_0", r_ns1, 0);
    check("poll_quick", el < 25000000L, 0);

    t0 = now_ns();
    expect("wait_30ms_returns_0", SC6(SYS_epoll_pwait2, ep, evs, 8, &t30, 0, 8), 0);
    el = now_ns() - t0;
    check("waited_at_least_30ms", el >= 30000000L, el);
    check("waited_less_than_2s", el < 2000000000L, el);
    check("timespec_unchanged", t30.tv_sec == 0 && t30.tv_nsec == 30000000, t30.tv_nsec);

    u64 one = 1;
    a_write(efd, &one, 8);
    t0 = now_ns();
    i64 r_null = SC6(SYS_epoll_pwait2, ep, evs, 8, 0, 0, 8);
    i64 r_big = SC6(SYS_epoll_pwait2, ep, evs, 8, &big, 0, 8);
    el = now_ns() - t0;
    expect("ready_null_timeout", r_null, 1);
    expect("ready_long_timeout", r_big, 1);
    check("ready_returns_at_once", el < 25000000L, 0);
    expect("ready_data", (i64)evs[0].data, 7);
    check("long_timespec_unchanged", big.tv_sec == 100000 && big.tv_nsec == 0, 0);
    a_read(efd, &one, 8);
    expect("drained", SC6(SYS_epoll_pwait2, ep, evs, 8, &zero, 0, 8), 0);

    /* Becomes ready while waiting without a limit. */
    i64 pid = a_spawn(0, &child_done, writer, 0);
    expect("woken_by_write", SC6(SYS_epoll_pwait2, ep, evs, 8, 0, 0, 8), 1);
    expect("woken_events", evs[0].events, EPOLLIN);
    a_spin_until(&child_done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    a_read(efd, &one, 8);

    /* And while waiting with a long limit. */
    pid = a_spawn(0, &child_done, writer, 0);
    t0 = now_ns();
    expect("woken_before_timeout", SC6(SYS_epoll_pwait2, ep, evs, 8, &big, 0, 8), 1);
    check("well_before_timeout", now_ns() - t0 < 10000000000L, 0);
    a_spin_until(&child_done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    return finish();
}
