/* A forked child reads the same system-wide clock parameters (tick,
 * tolerance, precision, constant, STA_NANO) as its parent, and without
 * capabilities a child is refused (EPERM) like anyone else.
 * poly-diff: syscalls=off (a child process; the clock state depends on the host's NTP)
 */
#include "a_sys.h"

#define SYS_capset 126
#define SYS_adjtimex 159
#define EPERM 1

/* struct timex, x86-64 (<linux/timex.h>): 208 bytes. */
struct timex {
    u32 modes;
    i32 hole0;
    i64 offset, freq, maxerror, esterror;
    i32 status;
    i32 hole1;
    i64 constant, precision, tolerance;
    i64 time_sec, time_usec;
    i64 tick, ppsfreq, jitter;
    i32 shift;
    i32 hole2;
    i64 stabil, jitcnt, calcnt, errcnt, stbcnt;
    i32 tai;
    i32 spare[11];
};

/* Mode bits and status bits (<linux/timex.h>). */
#define ADJ_OFFSET 0x0001
#define ADJ_FREQUENCY 0x0002
#define ADJ_MAXERROR 0x0004
#define ADJ_ESTERROR 0x0008
#define ADJ_STATUS 0x0010
#define ADJ_TIMECONST 0x0020
#define ADJ_TAI 0x0080
#define ADJ_SETOFFSET 0x0100
#define ADJ_MICRO 0x1000
#define ADJ_NANO 0x2000
#define ADJ_TICK 0x4000
#define ADJ_OFFSET_SINGLESHOT 0x8001
#define ADJ_OFFSET_SS_READ 0xa001
#define STA_PLL 0x0001
#define STA_PPSFREQ 0x0002
#define STA_PPSTIME 0x0004
#define STA_INS 0x0010
#define STA_DEL 0x0020
#define STA_UNSYNC 0x0040
#define STA_PPSSIGNAL 0x0100
#define STA_PPSJITTER 0x0200
#define STA_PPSWANDER 0x0400
#define STA_PPSERROR 0x0800
#define STA_CLOCKERR 0x1000
#define STA_NANO 0x2000
#define TIME_OK 0
#define TIME_ERROR 5

static A_UNUSED void fill(void *p, u8 v, size_t n) {
    u8 *b = p;
    for (size_t i = 0; i < n; i++) b[i] = v;
}

static A_UNUSED i64 adjtimex(void *buf) { return syscall1(SYS_adjtimex, buf); }

/* Empty all three capability sets (any user may): root then fails the
 * privilege checks too, and can never change the clock by accident. */
static A_UNUSED i64 drop_caps(void) {
    struct { u32 version; i32 pid; } h = { 0x20080522, 0 };
    struct { u32 effective, permitted, inheritable; } d[2];
    fill(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

int test_main(void) {
    struct timex p;
    fill(&p, 0, sizeof p);
    i64 r = adjtimex(&p);
    check("parent_read", r >= 0 && r <= 5, r);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct timex c, w;
        fill(&c, 0, sizeof c);
        i64 cr = adjtimex(&c);
        int code = 0;
        if (cr < 0 || cr > 5) code |= 1;
        if (c.tick != p.tick) code |= 2;
        if (c.tolerance != p.tolerance) code |= 4;
        if (c.precision != p.precision) code |= 8;
        if (c.constant != p.constant) code |= 16;
        if ((c.status & STA_NANO) != (p.status & STA_NANO)) code |= 32;
        drop_caps();
        fill(&w, 0, sizeof w);
        w.modes = ADJ_STATUS;
        if (adjtimex(&w) != -EPERM) code |= 64;
        exit_group(code);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    int code = (st & 0x7f) == 0 ? (st >> 8) & 0xff : 255;
    check("child_read", code != 255 && !(code & 1), code);
    check("child_tick", code != 255 && !(code & 2), code);
    check("child_tolerance", code != 255 && !(code & 4), code);
    check("child_precision", code != 255 && !(code & 8), code);
    check("child_constant", code != 255 && !(code & 16), code);
    check("child_nano", code != 255 && !(code & 32), code);
    check("child_write_eperm", code != 255 && !(code & 64), code);
    return finish();
}
