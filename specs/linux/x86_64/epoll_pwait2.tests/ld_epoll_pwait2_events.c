/* epoll_pwait2 reports ready file descriptors like epoll_wait: at most
 * maxevents records {events, data} (12 bytes each, packed), the count as
 * the result. Level-triggered entries are reported again while they stay
 * ready; an EPOLLET entry once per new readiness; EPOLLONESHOT once until
 * re-armed. With two ready fds and maxevents 1 the calls take turns. A
 * pipe whose writer is closed reports EPOLLHUP.
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_epoll_ctl 233
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_epoll_pwait2 441
/* linux/eventpoll.h */
#define EPOLL_CTL_ADD 1
#define EPOLL_CTL_MOD 3
#define EPOLLIN 0x001
#define EPOLLOUT 0x004
#define EPOLLHUP 0x010
#define EPOLLONESHOT (1U << 30)
#define EPOLLET (1U << 31)

struct epoll_event {
    u32 events;
    u64 data;
} __attribute__((packed));

static struct epoll_event evs[8];
static struct a_timespec zero = {0, 0};

static i64 wait_n(i64 ep, i64 max) {
    memset(evs, 0, sizeof evs);
    return SC6(SYS_epoll_pwait2, ep, evs, max, &zero, 0, 8);
}

static i64 add(i64 ep, i64 fd, u32 events, u64 data) {
    struct epoll_event e = {events, data};
    return SC6(SYS_epoll_ctl, ep, EPOLL_CTL_ADD, fd, &e, 0, 0);
}

int test_main(void) {
    check("event_size_12", sizeof(struct epoll_event) == 12, sizeof(struct epoll_event));
    i64 ep = syscall1(SYS_epoll_create1, 0);
    i64 a = syscall2(SYS_eventfd2, 0, 0), b = syscall2(SYS_eventfd2, 0, 0);
    add(ep, a, EPOLLIN, 100);
    add(ep, b, EPOLLIN, 200);
    u64 one = 1;
    a_write(a, &one, 8);
    a_write(b, &one, 8);
    expect("two_ready", wait_n(ep, 8), 2);
    expect("sum_of_data", (i64)(evs[0].data + evs[1].data), 300);
    expect("level_again", wait_n(ep, 8), 2);
    expect("max1_returns_1", wait_n(ep, 1), 1);
    u64 first = evs[0].data;
    expect("max1_again_returns_1", wait_n(ep, 1), 1);
    check("max1_takes_turns", evs[0].data != first, (i64)evs[0].data);
    check("rest_of_buffer_untouched", evs[1].events == 0 && evs[1].data == 0, (i64)evs[1].data);
    a_read(a, &one, 8);
    expect("one_left", wait_n(ep, 8), 1);
    expect("its_data", (i64)evs[0].data, 200);
    a_read(b, &one, 8);
    expect("none", wait_n(ep, 8), 0);

    /* Edge-triggered: once per write. */
    i64 c = syscall2(SYS_eventfd2, 0, 0);
    add(ep, c, EPOLLIN | EPOLLET, 300);
    a_write(c, &one, 8);
    expect("et_first", wait_n(ep, 8), 1);
    expect("et_events_in", evs[0].events, EPOLLIN);
    expect("et_not_again", wait_n(ep, 8), 0);
    a_write(c, &one, 8);
    expect("et_after_new_write", wait_n(ep, 8), 1);
    a_read(c, &one, 8);

    /* One-shot: disarmed after one report until EPOLL_CTL_MOD. */
    i64 d = syscall2(SYS_eventfd2, 0, 0);
    add(ep, d, EPOLLIN | EPOLLONESHOT, 400);
    a_write(d, &one, 8);
    expect("oneshot_first", wait_n(ep, 8), 1);
    expect("oneshot_disarmed", wait_n(ep, 8), 0);
    struct epoll_event m = {EPOLLIN | EPOLLONESHOT, 401};
    SC6(SYS_epoll_ctl, ep, EPOLL_CTL_MOD, d, &m, 0, 0);
    expect("oneshot_rearmed", wait_n(ep, 8), 1);
    expect("oneshot_new_data", (i64)evs[0].data, 401);
    a_read(d, &one, 8);

    /* Pipe: writable end reports EPOLLOUT; read end EPOLLHUP after close. */
    i32 p[2];
    syscall2(SYS_pipe2, p, 0);
    i64 ep2 = syscall1(SYS_epoll_create1, 0);
    add(ep2, p[1], EPOLLOUT, 1);
    expect("pipe_writable", wait_n(ep2, 8), 1);
    expect("pipe_out", evs[0].events, EPOLLOUT);
    i64 ep3 = syscall1(SYS_epoll_create1, 0);
    add(ep3, p[0], EPOLLIN, 2);
    expect("pipe_empty", wait_n(ep3, 8), 0);
    a_close(p[1]);
    expect("pipe_hup", wait_n(ep3, 8), 1);
    expect("pipe_hup_events", evs[0].events, EPOLLHUP);
    return finish();
}
