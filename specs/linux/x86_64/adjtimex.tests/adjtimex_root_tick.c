/* As root (CAP_SYS_TIME): ADJ_TICK outside 9000..11000 is EINVAL and
 * changes nothing; ADJ_TICK with the current tick succeeds and returns
 * the clock state; ADJ_SETOFFSET with a negative tv_usec, or tv_usec of
 * a whole second (microsecond mode), is EINVAL. Changes nothing else.
 * poly-diff: needs=root (only CAP_SYS_TIME may set clock parameters)
 * poly-diff: syscalls=names (the clock state depends on the host's NTP)
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
    struct timex t, cur;
    fill(&cur, 0, sizeof cur);
    adjtimex(&cur);
    i64 tick0 = cur.tick;
    fill(&t, 0, sizeof t);
    t.modes = ADJ_TICK;
    t.tick = 8999;
    i64 r = adjtimex(&t);
    if (r >= 0) { fill(&t, 0, sizeof t); t.modes = ADJ_TICK; t.tick = tick0; adjtimex(&t); }
    expect("tick_8999_einval", r, -EINVAL);
    fill(&t, 0, sizeof t);
    t.modes = ADJ_TICK;
    t.tick = 11001;
    r = adjtimex(&t);
    if (r >= 0) { fill(&t, 0, sizeof t); t.modes = ADJ_TICK; t.tick = tick0; adjtimex(&t); }
    expect("tick_11001_einval", r, -EINVAL);
    fill(&t, 0, sizeof t);
    adjtimex(&t);
    expect("tick_unchanged", t.tick, tick0);
    fill(&t, 0, sizeof t);
    t.modes = ADJ_TICK;
    t.tick = tick0;
    r = adjtimex(&t);
    check("same_tick_ok", r >= 0 && r <= 5, r);
    expect("same_tick_reads_back", t.tick, tick0);
    fill(&t, 0, sizeof t);
    t.modes = ADJ_SETOFFSET;
    t.time_sec = 0;
    t.time_usec = -1;
    expect("setoffset_negative_usec_einval", adjtimex(&t), -EINVAL);
    fill(&t, 0, sizeof t);
    t.modes = ADJ_SETOFFSET;
    t.time_sec = -1; /* would add nothing if it were taken */
    t.time_usec = 1000000;
    expect("setoffset_usec_whole_second_einval", adjtimex(&t), -EINVAL);
    return finish();
}
