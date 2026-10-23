/* pid 0, the caller's pid and the caller's tid all name the calling
 * (single-threaded) process. pid is a 32-bit int: the upper 32 bits of
 * the register are ignored (0x100000000 means 0, pid | 0x500000000 means
 * pid, 0x1ffffffff means -1: ESRCH). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getaffinity 204

static i64 get(i64 pid, u64 *out) {
    u64 m[2] = {0, 0x3333};
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    *out = m[1] == 0x3333 ? m[0] : 0;
    return r;
}

int test_main(void) {
    i64 pid = syscall0(SYS_getpid), tid = syscall0(SYS_gettid);
    u64 ref, v;
    expect("pid0", get(0, &ref), 8);
    check("pid0_mask", ref != 0, 0);
    expect("getpid", get(pid, &v), 8);
    check("getpid_same", v == ref, 0);
    expect("gettid", get(tid, &v), 8);
    check("gettid_same", v == ref, 0);
    expect("pid_4g_is_0", get(0x100000000L, &v), 8);
    check("pid_4g_same", v == ref, 0);
    expect("pid_high_bits", get(pid | 0x500000000L, &v), 8);
    check("pid_high_bits_same", v == ref, 0);
    expect("pid_1ffffffff", get(0x1ffffffffL, &v), -ESRCH);
    return finish();
}
