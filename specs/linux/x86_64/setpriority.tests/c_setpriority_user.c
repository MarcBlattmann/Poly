/* PRIO_USER sets the nice value of every process (every thread) whose
 * real uid is who; who 0 is the caller's real uid. In a child that
 * becomes uid 54321 with a second process: PRIO_USER 0 to 12 makes both
 * read 8; PRIO_USER 54321 to 14 makes both 6; lowering (to 5) is EACCES
 * for this unprivileged user and changes neither; a uid no process has
 * is ESRCH.
 * poly-diff: needs=root (sets up processes of another user)
 * poly-diff: syscalls=off (several processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
#define SYS_setrlimit 160
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_USER 2
/* asm-generic/resource.h */
#define RLIMIT_NICE 13

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static void body(void) {
    u64 lim[2] = { 0, 0 };
    i64 r = syscall2(SYS_setrlimit, RLIMIT_NICE, lim);
    if (r == 0) r = syscall2(SYS_setgroups, 0, 0);
    if (r == 0) r = syscall3(SYS_setresuid, 54321, 54321, 54321);
    check("became_54321", r == 0, r);
    volatile unsigned *g = shared_page(4096);
    i64 other = fork();
    if (other == 0) {
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(0);
    }
    gate_wait(&g[0]);
    r = setprio(PRIO_USER, 0, 12);
    check("user0_to_12", r == 0, r);
    r = getprio(PRIO_PROCESS, other);
    check("other_8", r == 8, r);
    r = getprio(PRIO_PROCESS, 0);
    check("self_8", r == 8, r);
    r = setprio(PRIO_USER, 54321, 14);
    check("user_to_14", r == 0, r);
    r = getprio(PRIO_PROCESS, other);
    check("other_6", r == 6, r);
    r = setprio(PRIO_USER, 0, 5);
    check("lowering_eacces", r == -EACCES, r);
    r = getprio(PRIO_USER, 0);
    check("all_still_6", r == 6, r);
    r = setprio(PRIO_USER, 0x3ffffff0, 15);
    check("unused_uid_esrch", r == -ESRCH, r);
    gate_open(&g[1]);
    wait_exit(other);
}

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        int before = failed;
        body();
        exit_group(failed != before);
    }
    int code = wait_exit(pid);
    check("child_checks", code == 0, code);
    return finish();
}
