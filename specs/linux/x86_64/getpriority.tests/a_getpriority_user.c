/* getpriority(PRIO_USER, uid) returns the highest value (lowest nice)
 * of all threads whose real uid is uid; who 0 is the caller's real uid.
 * Other processes of the same user may exist, so the result is only
 * known to be at least the value of every one of the caller's own
 * processes and threads; it is the same for who 0 and who = getuid().
 * poly-diff: syscalls=off (two processes; the uid is an argument)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_USER 2

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static int body(void) {
    i64 g0 = getprio(PRIO_PROCESS, 0);
    i64 n0 = 20 - g0;
    i64 uid = syscall0(SYS_getuid);
    i64 u = getprio(PRIO_USER, 0);
    check("in_range", u >= 1 && u <= 40, u);
    check("at_least_self", u >= g0, u);
    expect("uid_same_as_zero", getprio(PRIO_USER, uid), u);

    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        setprio(PRIO_PROCESS, 0, n0 + 1);
        sh[1] = 1;
        a_spin_until(&sh[0], 1, A_SPIN_LIMIT);
        syscall1(SYS_exit_group, 0);
    }
    check("child_ready", pid > 0 && a_spin_until(&sh[1], 1, A_SPIN_LIMIT), pid);
    expect("set_self_19", setprio(PRIO_PROCESS, 0, 19), 0);
    u = getprio(PRIO_USER, 0);
    check("at_least_child", u >= g0 - 1, u);
    expect("uid_same_as_zero_2", getprio(PRIO_USER, uid), u);
    sh[0] = 1;
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    u = getprio(PRIO_USER, 0);
    check("at_least_1", u >= 1 && u <= 40, u);
    expect("uid_with_no_process", getprio(PRIO_USER, 4000000), -ESRCH);
    return finish();
}

int test_main(void) { return a_in_session(body); }
