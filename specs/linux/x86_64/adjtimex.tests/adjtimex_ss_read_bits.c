/* modes with both ADJ_OFFSET_SINGLESHOT (0x8001) and ADJ_OFFSET_READONLY
 * (0x2000) set read the clock whatever other bits are set, also with no
 * capabilities (0xa003, 0xe001, 0xa0ff, 0x1a001, 0x8000a001), except
 * ADJ_SETOFFSET (0x0100): EPERM. Without the read-only bit (0x8001,
 * 0x8003) it is EPERM.
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

static int ok_read(u32 modes) {
    struct timex t;
    fill(&t, 0, sizeof t);
    t.modes = modes;
    i64 r = adjtimex(&t);
    return r >= 0 && r <= 5 && t.tolerance == 32768000 && t.modes == modes;
}

static i64 call(u32 modes) {
    struct timex t;
    fill(&t, 0, sizeof t);
    t.modes = modes;
    return adjtimex(&t);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    check("0xa003_reads", ok_read(0xa003), 0);
    check("0xe001_reads", ok_read(0xe001), 0);
    check("0xa0ff_reads", ok_read(0xa0ff), 0);
    check("0x1a001_reads", ok_read(0x1a001), 0);
    check("0x8000a001_reads", ok_read(0x8000a001u), 0);
    expect("0xa101_setoffset_eperm", call(0xa101), -EPERM);
    expect("0x8001_singleshot_eperm", call(ADJ_OFFSET_SINGLESHOT), -EPERM);
    expect("0x8003_eperm", call(0x8003), -EPERM);
    expect("0x2001_not_adjtime_eperm", call(0x2001), -EPERM);
    return finish();
}
