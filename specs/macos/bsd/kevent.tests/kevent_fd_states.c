#include "poly_mtest.h"
/* kevent, BSD 363: EVFILT_READ / EVFILT_WRITE state flags on pipes and sockets. Level triggered by default;
 * with EV_CLEAR an event is reported once per change and again only after new data arrives; EV_ONESHOT
 * removes it after one report even though the data is still unread; EV_DISPATCH reports once and disables
 * until EV_ENABLE; EV_DISABLE hides a ready event until EV_ENABLE; closing the descriptor drops its
 * registrations; on a socket pair whose peer closed, EVFILT_READ reports EV_EOF (0x8000) with data 0
 * and EVFILT_WRITE with EV_EOF too; EVFILT_WRITE on a pipe is ready with data > 0 (the room). */
#define EV_DISPATCH 0x80
#define SYS_socketpair 135
struct ts { i64 s, n; };
static struct ts zero;
static struct poly_kevent ch, ev[8];
static sysret kev(i64 kq, i64 nc, i64 ne) { return bsd6(SYS_kevent, kq, &ch, nc, ev, ne, &zero); }
static void set(u64 id, i16 filter, u16 flags) {
    memset(&ch, 0, sizeof ch);
    ch.ident = id; ch.filter = filter; ch.flags = flags;
}
int test_main(void) {
    sysret k = bsd0(SYS_kqueue);
    sysret p = bsd0(SYS_pipe);
    set(p.ret, EVFILT_READ, EV_ADD | EV_CLEAR);
    kev(k.ret, 1, 0);
    bsd3(SYS_write, p.ret2, "abc", 3);
    sysret r = kev(k.ret, 0, 8);
    check("EV_CLEAR: reported once with data 3", r.ret == 1 && ev[0].data == 3, r.ret);
    r = kev(k.ret, 0, 8);
    check("and not again though the data is unread", r.ret == 0, r.ret);
    bsd3(SYS_write, p.ret2, "de", 2);
    r = kev(k.ret, 0, 8);
    check("new data: reported again, data 5", r.ret == 1 && ev[0].data == 5, ev[0].data);
    set(p.ret, EVFILT_READ, EV_DELETE);
    kev(k.ret, 1, 0);
    set(p.ret, EVFILT_READ, EV_ADD | EV_ONESHOT);
    kev(k.ret, 1, 0);
    r = kev(k.ret, 0, 8);
    check("EV_ONESHOT: reported", r.ret == 1 && ev[0].flags == (EV_ADD | EV_ONESHOT), ev[0].flags);
    r = kev(k.ret, 0, 8);
    check("and removed though the data is unread", r.ret == 0, r.ret);
    set(p.ret, EVFILT_READ, EV_ADD | EV_DISPATCH);
    kev(k.ret, 1, 0);
    r = kev(k.ret, 0, 8);
    check("EV_DISPATCH: reported", r.ret == 1 && ev[0].flags == (EV_ADD | EV_DISPATCH), ev[0].flags);
    r = kev(k.ret, 0, 8);
    check("and then disabled", r.ret == 0, r.ret);
    set(p.ret, EVFILT_READ, EV_ENABLE);
    r = kev(k.ret, 1, 8);
    check("EV_ENABLE: reported again", r.ret == 1 && ev[0].data == 5, r.ret);
    set(p.ret, EVFILT_READ, EV_DISABLE);
    kev(k.ret, 1, 0);
    r = kev(k.ret, 0, 8);
    check("EV_DISABLE hides a ready event", r.ret == 0, r.ret);
    set(p.ret, EVFILT_READ, EV_ENABLE);
    r = kev(k.ret, 1, 8);
    check("EV_ENABLE shows it", r.ret == 1, r.ret);
    set(p.ret, EVFILT_READ, EV_ADD);
    r = kev(k.ret, 1, 8);
    value("EV_ADD of an existing registration: events returned by that call", r.ret);
    r = kev(k.ret, 0, 8);
    value("and the next call without changes", r.ret);
    set(p.ret2, EVFILT_WRITE, EV_ADD);
    r = kev(k.ret, 1, 8);
    int w = 0;
    for (int i = 0; i < r.ret; i++) if (ev[i].filter == EVFILT_WRITE) { w = 1; check("EVFILT_WRITE on a pipe: ready, data > 0", ev[i].data > 0, ev[i].data); }
    check("a write event arrived", w, r.ret);
    set(p.ret2, EVFILT_WRITE, EV_DELETE);
    kev(k.ret, 1, 0);
    bsd1(SYS_close, p.ret);
    set(p.ret, EVFILT_READ, EV_DELETE);
    r = kev(k.ret, 1, 8);
    check("closing the descriptor dropped the registration: delete is ENOENT or EBADF as an event", r.ret == 1 && (ev[0].flags & EV_ERROR) && (ev[0].data == ENOENT || ev[0].data == EBADF), ev[0].data);
    value("the error of deleting a closed descriptor's event", ev[0].data);
    int sv[2] = {0, 0};
    check_ok("socketpair", bsd4(SYS_socketpair, 1, 1, 0, sv));
    sysret k2 = bsd0(SYS_kqueue);
    set(sv[0], EVFILT_READ, EV_ADD | EV_CLEAR);
    kev(k2.ret, 1, 0);
    set(sv[0], EVFILT_WRITE, EV_ADD | EV_CLEAR);
    kev(k2.ret, 1, 0);
    bsd1(SYS_close, sv[1]);
    r = bsd6(SYS_kevent, k2.ret, 0, 0, ev, 8, &zero);
    int rd = 0, wr = 0;
    for (int i = 0; i < r.ret; i++) {
        if (ev[i].filter == EVFILT_READ && (ev[i].flags & EV_EOF) && ev[i].data == 0) rd = 1;
        if (ev[i].filter == EVFILT_WRITE && (ev[i].flags & EV_EOF)) wr = 1;
    }
    check("peer closed: EVFILT_READ with EV_EOF and data 0", rd, r.ret);
    check("and EVFILT_WRITE with EV_EOF", wr, r.ret);
    return finish();
}
