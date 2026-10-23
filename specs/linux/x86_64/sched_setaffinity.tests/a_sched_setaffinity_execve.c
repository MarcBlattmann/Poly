/* The CPU mask set by sched_setaffinity survives execve: the new program
 * starts with it (the highest usable CPU alone here). */
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
    u64 now = mask_of(0);
    if (a_argc() > 2 && str_eq(a_argv()[1], "again")) {
        passed = (int)a_atou(a_argv()[2]);
        u64 v = ~0UL;
        expect("set_after_execve", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
        check("mask_survives_execve", now != 0 && now == highest(mask_of(0)), 0);
        return finish();
    }
    u64 hi = highest(now);
    u64 v = hi;
    expect("set_highest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("set_seen", mask_of(0) == hi, 0);
    if (failed) return finish();
    char count[24];
    a_utoa(count, (u64)passed);
    char *argv[] = {"x", "again", count, 0};
    char *envp[] = {0};
    syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve_returned", 1, 0);
    return finish();
}
