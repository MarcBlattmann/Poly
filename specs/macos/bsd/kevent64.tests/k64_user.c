#include "poly_mtest.h"
/* kevent64, BSD 369 (kq, changelist, nchanges, eventlist, nevents, flags, timeout): like kevent with
 * 48-byte events {ident u64, filter i16, flags u16, fflags u32, data i64, udata u64, ext[2] u64}.
 * With EVFILT_USER (-10): a registered event (EV_ADD|EV_CLEAR) is reported after a change with
 * NOTE_TRIGGER (0x01000000) in fflags: one event, flags as added (0x21), data from the trigger's
 * change, udata and ext from the change that set them (a later change with udata 0 resets udata, ext
 * stays), nothing more until triggered again. The call takes 7 arguments and is tested on arm64 only. */
#if defined(POLY_ARM64)
#define SYS_kevent64 369
#define EVFILT_USER (-10)
#define NOTE_TRIGGER 0x01000000
#define NOTE_FFCOPY 0xc0000000u
#define NOTE_FFOR 0x80000000u
#define NOTE_FFAND 0x40000000u
struct k64 { u64 ident; i16 filter; u16 flags; u32 fflags; i64 data; u64 udata; u64 ext[2]; };
struct ts { i64 s, n; };
static struct k64 ch, ev[4];
static struct ts zero;
static sysret kv(i64 kq, i64 nc, i64 ne, u32 fl, struct ts *t) { return poly_bsd8(SYS_kevent64, kq, (i64)&ch, nc, (i64)ev, ne, fl, (i64)t, 0); }
static void set(u64 id, u16 flags, u32 fflags, i64 data, u64 udata, u64 e0, u64 e1) {
    memset(&ch, 0, sizeof ch);
    ch.ident = id; ch.filter = EVFILT_USER; ch.flags = flags; ch.fflags = fflags; ch.data = data; ch.udata = udata; ch.ext[0] = e0; ch.ext[1] = e1;
}
int test_main(void) {
    sysret k = bsd0(SYS_kqueue);
    set(7, EV_ADD | EV_CLEAR, 0, 0, 0x1122334455667788ul, 0xAAAA, 0xBBBB);
    sysret r = kv(k.ret, 1, 4, 0, &zero);
    check("add: no events", r.err == 0 && r.ret == 0, r.err);
    r = kv(k.ret, 0, 4, 0, &zero);
    check("nothing before the trigger", r.err == 0 && r.ret == 0, r.err);
    set(7, 0, NOTE_TRIGGER, 99, 0x1122334455667788ul, 0, 0);
    r = kv(k.ret, 1, 4, 0, &zero);
    check("a trigger: one event", r.err == 0 && r.ret == 1, r.err ? r.err : r.ret);
    check("ident 7, filter EVFILT_USER", ev[0].ident == 7 && ev[0].filter == EVFILT_USER, ev[0].filter);
    check("flags as added: EV_ADD|EV_CLEAR", ev[0].flags == (EV_ADD | EV_CLEAR), ev[0].flags);
    check("data 99 from the trigger", ev[0].data == 99, ev[0].data);
    check("fflags 0 (no fflag operation was given)", ev[0].fflags == 0, ev[0].fflags);
    check("the 64-bit udata is echoed whole", ev[0].udata == 0x1122334455667788ul, 0);
    check("ext values from the add", ev[0].ext[0] == 0xAAAA && ev[0].ext[1] == 0xBBBB, ev[0].ext[0]);
    r = kv(k.ret, 0, 4, 0, &zero);
    check("EV_CLEAR: nothing more", r.err == 0 && r.ret == 0, r.ret);
    set(7, 0, NOTE_TRIGGER, 5, 0, 0, 0);
    kv(k.ret, 1, 4, 0, &zero);
    check("a trigger whose change has udata 0: udata is 0 in the event", ev[0].udata == 0, 0);
    check("data 5", ev[0].data == 5, ev[0].data);
    check("ext unchanged", ev[0].ext[0] == 0xAAAA, ev[0].ext[0]);
    set(9, EV_ADD | EV_CLEAR, NOTE_FFCOPY | 0x0f, 0, 0, 0, 0);
    kv(k.ret, 1, 0, 0, &zero);
    set(9, 0, NOTE_TRIGGER | NOTE_FFOR | 0xf0, 0, 0, 0, 0);
    r = kv(k.ret, 1, 4, 0, &zero);
    check("fflags OR: low 24 bits 0x0f|0xf0 = 0xff, top bits 0xc0000000", r.ret == 1 && ev[0].fflags == 0xC00000FFu, ev[0].fflags);
    set(9, 0, NOTE_TRIGGER | NOTE_FFAND | 0x3c, 0, 0, 0, 0);
    r = kv(k.ret, 1, 4, 0, &zero);
    check("fflags AND 0x3c: 0x3c", r.ret == 1 && (ev[0].fflags & 0xffffff) == 0x3c, ev[0].fflags);
    set(7, EV_DELETE, 0, 0, 0, 0, 0);
    check_ok("delete", kv(k.ret, 1, 0, 0, &zero));
    set(7, 0, NOTE_TRIGGER, 0, 0, 0, 0);
    r = kv(k.ret, 1, 4, 0, &zero);
    check("triggering a deleted event: an error event ENOENT", r.ret == 1 && (ev[0].flags & EV_ERROR) && ev[0].data == ENOENT, ev[0].data);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
