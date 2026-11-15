/* kqueue, BSD 362, the descriptor: it is not a file (read is ENXIO, write
 * is ENXIO, lseek is ESPIPE), fstat shows mode 010000 (a FIFO type, no
 * permissions), a duplicate (dup2) is the same queue: an event registered
 * through one is reported through the other. */
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
    char b[4];
    check_err("read: ENXIO", bsd3(SYS_read, k.ret, b, 4), ENXIO);
    check_err("lseek: ESPIPE", bsd3(SYS_lseek, k.ret, 0, SEEK_SET), ESPIPE);
    sysret w = bsd3(SYS_write, k.ret, "x", 1);
    value("write: errno", w.err);
    struct poly_stat64 st;
    check_ok("fstat", bsd2(SYS_fstat64, k.ret, &st));
    check("mode 010000", st.st_mode == 010000, st.st_mode);
    sysret p = bsd0(SYS_pipe);
    set(p.ret, EVFILT_READ, EV_ADD, 0, 0, 0);
    kev(k.ret, 1, 0, &zero);
    sysret d = bsd2(SYS_dup2, k.ret, 20);
    check("dup2 of the queue", d.err == 0 && d.ret == 20, d.err);
    bsd3(SYS_write, p.ret2, "ab", 2);
    sysret r = bsd6(SYS_kevent, 20, 0, 0, ev, 4, &zero);
    check("an event registered through one is reported through the copy", r.err == 0 && r.ret == 1 && ev[0].data == 2, r.ret);
    return finish();
}
