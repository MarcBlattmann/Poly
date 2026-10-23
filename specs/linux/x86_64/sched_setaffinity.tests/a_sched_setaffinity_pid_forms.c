/* The pid argument: 0, getpid() and gettid() all name the calling
 * (single-threaded) process. pid is a 32-bit int: the upper 32 bits of
 * the register are ignored, so 0x100000000 is 0 and (N | 0x500000000) is
 * N, while 0x1ffffffff is -1 (ESRCH). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[16];
    memset(m, 0, sizeof m);
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

static u64 highest(u64 m) {
    u64 h = 0;
    for (; m; m &= m - 1) h = m & -m;
    return h;
}

int test_main(void) {
    u64 start = mask_of(0);
    u64 lo = start & -start, hi = highest(start);
    i64 pid = syscall0(SYS_getpid), tid = syscall0(SYS_gettid);

    u64 v = lo;
    expect("by_getpid", syscall3(SYS_sched_setaffinity, pid, 8, &v), 0);
    check("by_getpid_seen", mask_of(0) == lo, 0);
    v = hi;
    expect("by_gettid", syscall3(SYS_sched_setaffinity, tid, 8, &v), 0);
    check("by_gettid_seen", mask_of(pid) == hi, 0);
    v = lo;
    expect("upper_bits_zero", syscall3(SYS_sched_setaffinity, 0x100000000L, 8, &v), 0);
    check("upper_bits_zero_seen", mask_of(0) == lo, 0);
    v = hi;
    expect("upper_bits_pid", syscall3(SYS_sched_setaffinity, pid | 0x500000000L, 8, &v), 0);
    check("upper_bits_pid_seen", mask_of(0) == hi, 0);
    v = lo;
    expect("upper_bits_minus_one", syscall3(SYS_sched_setaffinity, 0x1ffffffffL, 8, &v), -ESRCH);
    check("failed_call_kept_mask", mask_of(0) == hi, 0);
    v = start;
    expect("restore", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    return finish();
}
