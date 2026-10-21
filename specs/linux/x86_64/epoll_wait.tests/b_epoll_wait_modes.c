/* epoll_wait triggering modes: level-triggered events are reported on
 * every call while the condition holds; EPOLLET reports once per new
 * arrival of data (not again while the old data sits there, again after
 * more is written); EPOLLONESHOT reports once and then nothing until
 * EPOLL_CTL_MOD re-arms it. With maxevents 1 and several ready
 * level-triggered descriptors, successive calls go round them in turn. */
#include "poly_ipc.h"

int test_main(void) {
    int e = (int)syscall1(SYS_epoll_create1, 0);
    int p[2];
    syscall2(SYS_pipe2, p, O_NONBLOCK);
    struct k_epoll_event ev = { EPOLLIN, 1 }, out[4];
    char buf[16];

    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[0], &ev);
    write(p[1], "ab", 2);
    i64 r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    i64 r2 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("level_reported_repeatedly", r1 == 1 && r2 == 1, r2);
    read(p[0], buf, 1);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("level_still_reported_with_data_left", r1 == 1, r1);
    read(p[0], buf, 1);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("level_quiet_when_drained", r1 == 0, r1);

    /* Edge-triggered. */
    ev.events = EPOLLIN | EPOLLET;
    ev.data = 2;
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[0], &ev);
    write(p[1], "cd", 2);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    r2 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("edge_reported_once", r1 == 1 && r2 == 0, r1 * 10 + r2);
    read(p[0], buf, 1);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("edge_partial_read_no_new_event", r1 == 0, r1);
    write(p[1], "e", 1);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("edge_new_data_new_event", r1 == 1 && out[0].events == EPOLLIN, r1);
    while (read(p[0], buf, sizeof buf) > 0) {
    }

    /* The MOD itself re-checks: data already there is reported after MOD. */
    write(p[1], "f", 1);
    syscall4(SYS_epoll_wait, e, out, 4, 0);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[0], &ev);
    r2 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("mod_rearms_edge", r1 == 0 && r2 == 1, r1 * 10 + r2);
    while (read(p[0], buf, sizeof buf) > 0) {
    }

    /* One-shot. */
    ev.events = EPOLLIN | EPOLLONESHOT;
    ev.data = 3;
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[0], &ev);
    write(p[1], "g", 1);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    r2 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("oneshot_once", r1 == 1 && r2 == 0, r1 * 10 + r2);
    write(p[1], "h", 1);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("oneshot_disabled_until_mod", r1 == 0, r1);
    i64 r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[0], &ev);
    check("disabled_still_registered_eexist", r == -EEXIST, r);
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[0], &ev);
    r1 = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("oneshot_rearmed_by_mod", r1 == 1 && out[0].data == 3, r1);
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_DEL, p[0], 0);

    /* Round robin with maxevents 1: three writable pipes. */
    int q[3][2];
    for (int i = 0; i < 3; i++) {
        syscall2(SYS_pipe2, q[i], 0);
        struct k_epoll_event w = { EPOLLOUT, (u64)(100 + i) };
        syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, q[i][1], &w);
    }
    u64 seen[6];
    for (int i = 0; i < 6; i++) {
        syscall4(SYS_epoll_wait, e, out, 1, 0);
        seen[i] = out[0].data;
    }
    int distinct = seen[0] != seen[1] && seen[1] != seen[2] && seen[0] != seen[2];
    int cycles = seen[3] == seen[0] && seen[4] == seen[1] && seen[5] == seen[2];
    check("maxevents_1_visits_each", distinct, (i64)seen[1]);
    check("maxevents_1_cycles", cycles, (i64)seen[3]);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("all_three_at_once", r == 3, r);
    return finish();
}
