/* setpriority(PRIO_PROCESS, 0, nice) sets the caller's nice value and
 * returns 0; any caller may raise it (lower its priority). Values above
 * 19 are clamped to 19 (100 -> getpriority 1), not rejected; setting
 * the value it already has succeeds. The nice argument is a 32-bit int
 * (1<<32 | 5 is 5). A fork child inherits the nice value. All in a
 * child, so the test program itself stays at nice 0.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static void body(void) {
    i64 r = setprio(PRIO_PROCESS, 0, (1L << 32) | 5);
    check("set_5_high_bits_ignored", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("reads_15", r == 15, r);
    r = setprio(PRIO_PROCESS, 0, 5);
    check("same_value_ok", r == 0, r);
    r = setprio(PRIO_PROCESS, getpid(), 7);
    check("by_own_pid_7", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("reads_13", r == 13, r);
    i64 c = fork();
    if (c == 0) exit_group(getprio(PRIO_PROCESS, 0) == 13 ? 0 : 1);
    check("child_inherits_7", wait_exit(c) == 0, 0);
    r = setprio(PRIO_PROCESS, 0, 100);
    check("set_100_ok", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("clamped_to_19", r == 1, r);
    r = setprio(PRIO_PROCESS, 0, 0x7fffffff);
    check("int_max_ok", r == 0, r);
    r = setprio(PRIO_PROCESS, 0, 19);
    check("19_again_ok", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("still_19", r == 1, r);
}

int test_main(void) {
    i64 r = getprio(PRIO_PROCESS, 0);
    check("start_20", r == 20, r);
    i64 pid = fork();
    if (pid == 0) {
        int before = failed;
        body();
        exit_group(failed != before);
    }
    int code = wait_exit(pid);
    check("child_checks", code == 0, code);
    r = getprio(PRIO_PROCESS, 0);
    check("parent_still_20", r == 20, r);
    return finish();
}
