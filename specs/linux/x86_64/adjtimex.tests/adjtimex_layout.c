/* adjtimex reads and writes exactly 208 bytes: a buffer ending right at
 * an inaccessible page works; modes comes back as the caller wrote it,
 * and the alignment holes and the 44 spare bytes after tai keep the
 * caller's bytes; every other field is overwritten.
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
    u8 *m = (u8 *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall3(SYS_mprotect, m + 4096, 4096, PROT_NONE);
    struct timex *t = (struct timex *)(m + 4096 - sizeof(struct timex));
    expect("struct_size", sizeof(struct timex), 208);
    fill(t, 0xa5, sizeof *t);
    t->modes = 0;
    i64 r = adjtimex(t);
    check("ends_at_page_end_ok", r >= 0 && r <= 5, r);
    expect("modes_unchanged", t->modes, 0);
    expect("hole0_kept", (u32)t->hole0, 0xa5a5a5a5u);
    expect("hole1_kept", (u32)t->hole1, 0xa5a5a5a5u);
    expect("hole2_kept", (u32)t->hole2, 0xa5a5a5a5u);
    int spare_ok = 1;
    for (int i = 0; i < 11; i++) spare_ok &= (u32)t->spare[i] == 0xa5a5a5a5u;
    check("spare_kept", spare_ok, t->spare[0]);
    check("tick_written", t->tick != (i64)0xa5a5a5a5a5a5a5a5ULL, t->tick);
    check("tolerance_written", t->tolerance == 32768000, t->tolerance);
    check("stbcnt_written", t->stbcnt == 0, t->stbcnt);
    check("tai_written", t->tai >= 0, t->tai);
    check("time_written", t->time_sec > 1577836800L, t->time_sec);
    return finish();
}
