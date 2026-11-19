#include "poly_mtest.h"
/* kevent64, BSD 369: the flags argument and error reporting. KEVENT_FLAG_IMMEDIATE (1) polls even
 * with a NULL timeout; KEVENT_FLAG_ERROR_EVENTS (2) reports only change errors: a good change gives
 * 0 events, a bad one an event with EV_ERROR (0x4000) added to its flags, data the errno (EINVAL for an
 * unknown filter), udata echoed; with no room for events the call fails with that errno. Unknown flag
 * bits are ignored; a closed queue is EBADF (9). arm64 only (seven arguments). */
#if defined(POLY_ARM64)
#define SYS_kevent64 369
#define EVFILT_USER (-10)
struct k64 { u64 ident; i16 filter; u16 flags; u32 fflags; i64 data; u64 udata; u64 ext[2]; };
struct ts { i64 s, n; };
static struct k64 ch, ev[4];
static struct ts zero;
static sysret kv(i64 kq, i64 nc, i64 ne, u32 fl, struct ts *t) { return poly_bsd8(SYS_kevent64, kq, (i64)&ch, nc, (i64)ev, ne, fl, (i64)t, 0); }
int test_main(void) {
    sysret k = bsd0(SYS_kqueue);
    memset(&ch, 0, sizeof ch);
    sysret r = kv(k.ret, 0, 4, 1, 0);
    check("IMMEDIATE with a NULL timeout polls: 0 events", r.err == 0 && r.ret == 0, r.err);
    ch.ident = 7; ch.filter = EVFILT_USER; ch.flags = EV_ADD;
    r = kv(k.ret, 1, 4, 2, &zero);
    check("ERROR_EVENTS: a good change reports nothing", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    ch.ident = 1000; ch.filter = -50; ch.flags = EV_ADD; ch.udata = 77;
    r = kv(k.ret, 1, 4, 2, &zero);
    check("a bad change reports one event", r.err == 0 && r.ret == 1, r.err ? r.err : r.ret);
    check("flags: EV_ERROR plus EV_ADD", ev[0].flags == (EV_ERROR | EV_ADD), ev[0].flags);
    check("data is EINVAL", ev[0].data == EINVAL, ev[0].data);
    check("ident, filter and udata are echoed", ev[0].ident == 1000 && ev[0].filter == -50 && ev[0].udata == 77, ev[0].ident);
    r = kv(k.ret, 1, 0, 0, &zero);
    check_err("no room for the error event: the call fails with EINVAL", r, EINVAL);
    r = kv(k.ret, 1, 4, 0, &zero);
    check("without ERROR_EVENTS the same error is an event too", r.err == 0 && r.ret == 1 && (ev[0].flags & EV_ERROR) && ev[0].data == EINVAL, r.err ? r.err : r.ret);
    ch.ident = 5; ch.filter = EVFILT_READ; ch.flags = EV_ADD; ch.udata = 3;
    r = kv(k.ret, 1, 4, 0, &zero);
    check("EVFILT_READ on a descriptor that is not open: EBADF as an event", r.err == 0 && r.ret == 1 && ev[0].data == EBADF && ev[0].ident == 5, r.err ? r.err : ev[0].data);
    r = kv(k.ret, 0, 4, 0x100, &zero);
    check("unknown flag bits are ignored", r.err == 0 && r.ret == 0, r.err);
    r = kv(99, 0, 4, 0, &zero);
    check_err("a descriptor that is not open is EBADF", r, EBADF);
    sysret p = bsd0(SYS_pipe);
    r = kv(p.ret, 0, 4, 0, &zero);
    check_err("a pipe is not a queue: EBADF", r, EBADF);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
