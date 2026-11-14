/* kqueue, BSD 362, and fork: the child has the descriptor number but not
 * the queue's registrations. Values: what the child's kevent on the parent's
 * queue returns (an event count, or an errno). */
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
    sysret p = bsd0(SYS_pipe);
    set(p.ret, EVFILT_READ, EV_ADD, 0, 0, 0);
    kev(k.ret, 1, 0, &zero);
    bsd3(SYS_write, p.ret2, "ab", 2);
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        sysret r = bsd6(SYS_kevent, k.ret, 0, 0, ev, 4, &zero);
        poly_exit(r.err ? 100 + (int)r.err : (int)r.ret);
    }
    int st = poly_wait(f.ret);
    check("the child exited", W_EXITED(st), st);
    value("the child's kevent: events (0..3) or 100 + errno", W_EXITSTATUS(st));
    sysret r = kev(k.ret, 0, 4, &zero);
    check("the parent's queue still has its event", r.err == 0 && r.ret == 1, r.ret);
    return finish();
}
