/* kevent, BSD 363: EVFILT_TIMER (data in milliseconds) with EV_ONESHOT fires
 * once after the time with data = 1 expiration, the flags as added; EVFILT_SIGNAL
 * counts signals sent to the process, also ignored ones (two kills: data 2),
 * and starts again from 0; waiting with a timeout and nothing to report
 * returns 0 after the time. */
#include "poly_mtest.h"

struct ts {
    i64 sec, nsec;
};
static struct ts zero = {0, 0};
static struct poly_kevent ch, ev[4];
static sysret kev(i64 kq, i64 nchanges, i64 nevents, struct ts *timeout) {
    return bsd6(SYS_kevent, kq, &ch, nchanges, ev, nevents, timeout);
}
static void set(u64 ident, i16 filter, u16 flags, u32 fflags, i64 data, i64 udata) {
    ch.ident = ident;
    ch.filter = filter;
    ch.flags = flags;
    ch.fflags = fflags;
    ch.data = data;
    ch.udata = (void *)udata;
}

int test_main(void) {
    sysret k = bsd0(SYS_kqueue);
    set(1, EVFILT_TIMER, EV_ADD | EV_ONESHOT, 0, 20, 4);
    sysret r = kev(k.ret, 1, 0, &zero);
    check("add a 20 ms one-shot timer", r.err == 0, r.err);
    struct ts one = {1, 0};
    r = kev(k.ret, 0, 4, &one);
    check("waiting up to 1 s: the timer fires", r.err == 0 && r.ret == 1, r.err ? r.err : r.ret);
    check("ident 1, EVFILT_TIMER, 1 expiration", ev[0].ident == 1 && ev[0].filter == EVFILT_TIMER && ev[0].data == 1, ev[0].data);
    check("flags as added (EV_ADD|EV_ONESHOT|EV_CLEAR)", ev[0].flags == (EV_ADD | EV_ONESHOT | EV_CLEAR), ev[0].flags);
    check("udata echoed", ev[0].udata == (void *)4, 0);
    r = kev(k.ret, 0, 4, &zero);
    check("a one-shot timer is gone afterwards", r.err == 0 && r.ret == 0, r.ret);
    struct ts t50 = {0, 50000000};
    r = kev(k.ret, 0, 4, &t50);
    check("a 50 ms wait with nothing to report: 0", r.err == 0 && r.ret == 0, r.ret);
    poly_sigaction(SIGUSR1, SIG_IGN, 0, 0, 0);
    set(SIGUSR1, EVFILT_SIGNAL, EV_ADD, 0, 0, 8);
    kev(k.ret, 1, 0, &zero);
    i64 me = bsd0(SYS_getpid).ret;
    bsd2(SYS_kill, me, SIGUSR1);
    bsd2(SYS_kill, me, SIGUSR1);
    r = kev(k.ret, 0, 4, &zero);
    check("two ignored SIGUSR1: one event", r.err == 0 && r.ret == 1, r.ret);
    check("ident 30, EVFILT_SIGNAL, data 2", ev[0].ident == 30 && ev[0].filter == EVFILT_SIGNAL && ev[0].data == 2, ev[0].data);
    r = kev(k.ret, 0, 4, &zero);
    check("reported once: then nothing", r.err == 0 && r.ret == 0, r.ret);
    return finish();
}
