/* kevent, BSD 363, errors: a bad change is reported as an event with
 * EV_ERROR and data = errno when there is room for events (EBADF for a
 * descriptor that is not open, EINVAL for an unknown filter), else the call
 * fails with that errno; EV_RECEIPT reports every change as an EV_ERROR event
 * with data 0; EBADF for a queue that is not open and for a descriptor that
 * is not a queue; EFAULT for the change list; EINVAL for an invalid timeout. */
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
    set(99, EVFILT_READ, EV_ADD, 0, 0, 1);
    sysret r = kev(k.ret, 1, 4, &zero);
    check("a bad descriptor with room: one event", r.err == 0 && r.ret == 1, r.err ? r.err : r.ret);
    check("EV_ERROR|EV_ADD, data EBADF, ident and udata echoed", ev[0].flags == (EV_ERROR | EV_ADD) && ev[0].data == EBADF && ev[0].ident == 99 && ev[0].udata == (void *)1, ev[0].data);
    check_err("without room: EBADF", kev(k.ret, 1, 0, &zero), EBADF);
    set(p.ret, 99, EV_ADD, 0, 0, 1);
    r = kev(k.ret, 1, 4, &zero);
    check("an unknown filter: an event with data EINVAL", r.ret == 1 && ev[0].flags == (EV_ERROR | EV_ADD) && ev[0].data == EINVAL, ev[0].data);
    set(p.ret, EVFILT_READ, EV_ADD | EV_RECEIPT, 0, 0, 9);
    r = kev(k.ret, 1, 4, &zero);
    check("EV_RECEIPT: one event for the successful change", r.err == 0 && r.ret == 1, r.ret);
    check("flags EV_ERROR|EV_RECEIPT|EV_ADD, data 0", ev[0].flags == (EV_ERROR | EV_RECEIPT | EV_ADD) && ev[0].data == 0, ev[0].flags);
    check_err("a queue that is not open: EBADF", bsd6(SYS_kevent, 99, &ch, 0, ev, 4, &zero), EBADF);
    check_err("a descriptor that is not a queue: EBADF", bsd6(SYS_kevent, p.ret, &ch, 0, ev, 4, &zero), EBADF);
    check_err("the change list at address 8: EFAULT", bsd6(SYS_kevent, k.ret, (void *)8, 1, ev, 4, &zero), EFAULT);
    struct ts bad = {0, 2000000000}, neg = {-1, 0};
    check_err("a timeout of 2e9 ns: EINVAL", bsd6(SYS_kevent, k.ret, &ch, 0, ev, 4, &bad), EINVAL);
    check_err("a negative timeout: EINVAL", bsd6(SYS_kevent, k.ret, &ch, 0, ev, 4, &neg), EINVAL);
    return finish();
}
