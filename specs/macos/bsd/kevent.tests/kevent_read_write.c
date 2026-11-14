/* kevent, BSD 363 (kq, changelist, nchanges, eventlist, nevents, timeout):
 * EVFILT_READ on a pipe reports nothing while it is empty and, once 3 bytes
 * are in it, one event with the ident, the filter, the flags as added
 * (EV_ADD), data = 3 and the udata; the event is level triggered (reported
 * again until the data is read); EVFILT_WRITE reports room to write; EV_DELETE
 * removes a registration, and deleting it again fails with ENOENT. */
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
    char b[8];
    set(p.ret, EVFILT_READ, EV_ADD, 0, 0, 77);
    sysret r = kev(k.ret, 1, 4, &zero);
    check("EV_ADD of an empty pipe: no event yet", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    bsd3(SYS_write, p.ret2, "abc", 3);
    r = kev(k.ret, 0, 4, &zero);
    check("after a write: one event", r.err == 0 && r.ret == 1, r.ret);
    check("ident is the descriptor, filter EVFILT_READ", ev[0].ident == (u64)p.ret && ev[0].filter == EVFILT_READ, ev[0].filter);
    check("flags are EV_ADD, data 3, udata echoed", ev[0].flags == EV_ADD && ev[0].data == 3 && ev[0].udata == (void *)77, ev[0].flags);
    r = kev(k.ret, 0, 4, &zero);
    check("level triggered: reported again", r.ret == 1 && ev[0].data == 3, r.ret);
    bsd3(SYS_read, p.ret, b, 8);
    r = kev(k.ret, 0, 4, &zero);
    check("after reading: nothing", r.err == 0 && r.ret == 0, r.ret);
    set(p.ret2, EVFILT_WRITE, EV_ADD, 0, 0, 5);
    r = kev(k.ret, 1, 4, &zero);
    check("EVFILT_WRITE on the write end: ready at once", r.err == 0 && r.ret == 1 && ev[0].filter == EVFILT_WRITE, r.ret);
    check("with room (data > 0)", ev[0].data > 0, ev[0].data);
    set(p.ret2, EVFILT_WRITE, EV_DELETE, 0, 0, 0);
    check_ok("EV_DELETE", kev(k.ret, 1, 0, &zero));
    check_err("EV_DELETE again, no room for events: ENOENT", kev(k.ret, 1, 0, &zero), ENOENT);
    r = kev(k.ret, 1, 4, &zero);
    check("with room for events: reported as an event", r.err == 0 && r.ret == 1, r.ret);
    check("flags EV_ERROR|EV_DELETE, data ENOENT", ev[0].flags == (EV_ERROR | EV_DELETE) && ev[0].data == ENOENT, ev[0].data);
    return finish();
}
