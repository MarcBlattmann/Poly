#include "poly_mtest.h"
/* kevent, BSD 363: EVFILT_USER (-10) and the event states. A user event added with EV_ADD|EV_CLEAR
 * is reported once after a change with NOTE_TRIGGER (0x01000000), flags as added (0x21), data from the
 * trigger's change; EV_CLEAR: not reported again until triggered again; without EV_CLEAR a triggered
 * event stays triggered (reported by every call) until it is deleted or cleared with EV_CLEAR; fflags
 * operations (NOTE_FFCOPY 0xc0000000, FFOR 0x80000000, FFAND 0x40000000) combine the low 24 bits and
 * the event reports the result with the top bits 0xc0000000; EV_DISPATCH (0x80) reports once and
 * disables the event until EV_ENABLE; EV_DISABLE keeps a trigger pending until EV_ENABLE; EV_ONESHOT
 * deletes the event after one report; adding the same ident twice keeps one event; a trigger of an event
 * that was never added is an error event with data ENOENT (2). */
#define EVFILT_USER (-10)
#define NOTE_TRIGGER 0x01000000u
#define NOTE_FFCOPY 0xc0000000u
#define NOTE_FFOR 0x80000000u
#define NOTE_FFAND 0x40000000u
#define EV_DISPATCH 0x80
struct ts { i64 s, n; };
static struct ts zero;
static struct poly_kevent ch, ev[8];
static sysret kev(i64 kq, i64 nc, i64 ne) { return bsd6(SYS_kevent, kq, &ch, nc, ev, ne, &zero); }
static void set(u64 id, u16 flags, u32 fflags, i64 data, i64 udata) {
    memset(&ch, 0, sizeof ch);
    ch.ident = id; ch.filter = EVFILT_USER; ch.flags = flags; ch.fflags = fflags; ch.data = data; ch.udata = (void *)udata;
}
int test_main(void) {
    sysret k = bsd0(SYS_kqueue);
    set(1, EV_ADD | EV_CLEAR, 0, 0, 11);
    sysret r = kev(k.ret, 1, 8);
    check("add: no events", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    set(1, 0, NOTE_TRIGGER, 42, 11);
    r = kev(k.ret, 1, 8);
    check("trigger: one event", r.err == 0 && r.ret == 1, r.err ? r.err : r.ret);
    check("ident 1, EVFILT_USER, flags 0x21, data 42, udata echoed", ev[0].ident == 1 && ev[0].filter == EVFILT_USER && ev[0].flags == (EV_ADD | EV_CLEAR) && ev[0].data == 42 && ev[0].udata == (void *)11, ev[0].flags);
    r = kev(k.ret, 0, 8);
    check("EV_CLEAR: not reported again", r.err == 0 && r.ret == 0, r.ret);
    set(2, EV_ADD, 0, 0, 0);
    kev(k.ret, 1, 8);
    set(2, 0, NOTE_TRIGGER, 0, 0);
    kev(k.ret, 1, 8);
    r = kev(k.ret, 0, 8);
    check("without EV_CLEAR a triggered event is reported again", r.err == 0 && r.ret == 1 && ev[0].ident == 2, r.ret);
    r = kev(k.ret, 0, 8);
    check("and again", r.err == 0 && r.ret == 1, r.ret);
    set(2, EV_ENABLE | EV_CLEAR, 0, 0, 0);
    set(2, EV_DELETE, 0, 0, 0);
    kev(k.ret, 1, 8);
    r = kev(k.ret, 0, 8);
    check("after a delete: nothing", r.err == 0 && r.ret == 0, r.ret);
    set(3, EV_ADD | EV_CLEAR, NOTE_FFCOPY | 0x0f, 0, 0);
    kev(k.ret, 1, 0);
    set(3, 0, NOTE_TRIGGER | NOTE_FFOR | 0xf0, 0, 0);
    r = kev(k.ret, 1, 8);
    check("FFOR: fflags 0xc00000ff", r.ret == 1 && ev[0].fflags == 0xC00000FFu, ev[0].fflags);
    set(3, 0, NOTE_TRIGGER | NOTE_FFAND | 0x3c, 0, 0);
    r = kev(k.ret, 1, 8);
    check("FFAND 0x3c: low bits 0x3c", r.ret == 1 && (ev[0].fflags & 0xffffff) == 0x3c, ev[0].fflags);
    set(3, 0, NOTE_TRIGGER | NOTE_FFCOPY | 0x5, 0, 0);
    r = kev(k.ret, 1, 8);
    check("FFCOPY 5: low bits 5", r.ret == 1 && (ev[0].fflags & 0xffffff) == 5, ev[0].fflags);
    set(4, EV_ADD | EV_DISPATCH, 0, 0, 0);
    kev(k.ret, 1, 0);
    set(4, 0, NOTE_TRIGGER, 0, 0);
    r = kev(k.ret, 1, 8);
    check("EV_DISPATCH: reported, flags 0x81", r.ret == 1 && ev[0].flags == (EV_ADD | EV_DISPATCH), ev[0].flags);
    set(4, 0, NOTE_TRIGGER, 0, 0);
    r = kev(k.ret, 1, 8);
    check("a trigger while it is disabled reports nothing", r.err == 0 && r.ret == 0, r.ret);
    set(4, EV_ENABLE, 0, 0, 0);
    r = kev(k.ret, 1, 8);
    check("EV_ENABLE delivers the pending trigger", r.err == 0 && r.ret == 1, r.ret);
    set(5, EV_ADD | EV_CLEAR, 0, 0, 0);
    kev(k.ret, 1, 0);
    set(5, EV_DISABLE, 0, 0, 0);
    kev(k.ret, 1, 0);
    set(5, 0, NOTE_TRIGGER, 0, 0);
    r = kev(k.ret, 1, 8);
    check("a disabled event is not reported", r.err == 0 && r.ret == 0, r.ret);
    set(5, EV_ENABLE, 0, 0, 0);
    r = kev(k.ret, 1, 8);
    check("after EV_ENABLE the trigger is reported", r.err == 0 && r.ret == 1, r.ret);
    set(6, EV_ADD | EV_ONESHOT, 0, 0, 0);
    kev(k.ret, 1, 0);
    set(6, 0, NOTE_TRIGGER, 0, 0);
    r = kev(k.ret, 1, 8);
    check("EV_ONESHOT reports once", r.ret == 1, r.ret);
    set(6, 0, NOTE_TRIGGER, 0, 0);
    r = kev(k.ret, 1, 8);
    check("then the event is gone: a trigger is an error event ENOENT", r.ret == 1 && (ev[0].flags & EV_ERROR) && ev[0].data == ENOENT, ev[0].data);
    set(7, EV_ADD | EV_CLEAR, 0, 0, 1);
    kev(k.ret, 1, 0);
    set(7, EV_ADD | EV_CLEAR, 0, 0, 2);
    kev(k.ret, 1, 0);
    set(7, 0, NOTE_TRIGGER, 0, 2);
    r = kev(k.ret, 1, 8);
    check("adding an ident twice keeps one event, with the later udata", r.ret == 1 && ev[0].udata == (void *)2, r.ret);
    set(7, 0, NOTE_TRIGGER, 0, 2);
    kev(k.ret, 1, 0);
    set(99, 0, NOTE_TRIGGER, 0, 0);
    r = kev(k.ret, 1, 8);
    check("triggering an event never added: an error event with ENOENT", r.ret >= 1 && (ev[r.ret - 1].flags & EV_ERROR) && ev[r.ret - 1].data == ENOENT, r.ret);
    return finish();
}
