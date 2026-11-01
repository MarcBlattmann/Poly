/* Lowering the nice value (raising priority) needs CAP_SYS_NICE or
 * RLIMIT_NICE: a caller may set nice n if 20 - n <= RLIMIT_NICE (the
 * limit on the 1..40 scale). A child that drops to uid 65534:
 * - with RLIMIT_NICE 0: nice 5 -> 3 is EACCES, -100 (clamped to -20)
 *   is EACCES, 5 again and 8 are fine; a failed call changes nothing;
 * - with RLIMIT_NICE 25: down to -5 is allowed, -6 is EACCES.
 * Root (CAP_SYS_NICE) may go down to -20; -100 clamps to -20 (40).
 * poly-diff: needs=root (negative nice values, resource limits and dropping to another user)
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
#define SYS_setrlimit 160
/* linux/resource.h */
#define PRIO_PROCESS 0
/* asm-generic/resource.h */
#define RLIMIT_NICE 13

static i64 getprio(void) { return syscall2(SYS_getpriority, PRIO_PROCESS, 0); }
static i64 setnice(i64 nice) { return syscall3(SYS_setpriority, PRIO_PROCESS, 0, nice); }
static i64 set_rlimit(int res, u64 cur, u64 max) {
    u64 r[2] = { cur, max };
    return syscall2(SYS_setrlimit, res, r);
}
static i64 become(unsigned id) {
    i64 r = syscall2(SYS_setgroups, 0, 0);
    if (r == 0) r = syscall3(SYS_setresgid, id, id, id);
    if (r == 0) r = syscall3(SYS_setresuid, id, id, id);
    return r;
}
static void in_child(const char *name, void (*fn)(void)) {
    i64 pid = fork();
    if (pid == 0) {
        int before = failed;
        fn();
        exit_group(failed != before);
    }
    int code = wait_exit(pid);
    check(name, code == 0, code);
}

static void no_limit(void) {
    i64 r = set_rlimit(RLIMIT_NICE, 0, 0);
    if (r == 0) r = become(65534);
    check("dropped", r == 0, r);
    r = setnice(5);
    check("raise_to_5_ok", r == 0, r);
    r = setnice(3);
    check("lower_to_3_eacces", r == -EACCES, r);
    r = setnice(-100);
    check("lower_to_minus100_eacces", r == -EACCES, r);
    r = getprio();
    check("still_5", r == 15, r);
    r = setnice(5);
    check("same_ok", r == 0, r);
    r = setnice(8);
    check("raise_to_8_ok", r == 0, r);
}

static void limit_25(void) {
    i64 r = set_rlimit(RLIMIT_NICE, 25, 25);
    if (r == 0) r = become(65534);
    check("dropped", r == 0, r);
    r = setnice(-5);
    check("minus5_ok", r == 0, r);
    r = getprio();
    check("reads_25", r == 25, r);
    r = setnice(-6);
    check("minus6_eacces", r == -EACCES, r);
    r = setnice(-100);
    check("minus100_eacces", r == -EACCES, r);
    r = setnice(0);
    check("back_to_0_ok", r == 0, r);
    r = setnice(-3);
    check("minus3_ok", r == 0, r);
}

static void privileged(void) {
    i64 r = setnice(-20);
    check("minus20_ok", r == 0, r);
    r = getprio();
    check("reads_40", r == 40, r);
    r = setnice(10);
    check("raise_10", r == 0, r);
    r = setnice(-100);
    check("minus100_ok", r == 0, r);
    r = getprio();
    check("clamped_minus20", r == 40, r);
}

int test_main(void) {
    in_child("no_limit", no_limit);
    in_child("limit_25", limit_25);
    in_child("privileged", privileged);
    return finish();
}
