/* Changing another process's nice value without CAP_SYS_NICE: allowed
 * only if the caller's effective uid is the target's real or effective
 * uid (target: real 1111, effective 2222, saved 3333), else EPERM, even
 * when nothing would change and before the EACCES check for lowering.
 * A target holding capabilities the caller lacks is EPERM even with
 * matching uids. Root may change anyone.
 * poly-diff: needs=root (sets up processes of several users)
 * poly-diff: syscalls=off (several processes)
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

static i64 setprio(i64 who, i64 nice) { return syscall3(SYS_setpriority, PRIO_PROCESS, who, nice); }

static i64 try_as(i64 target, unsigned r, unsigned e, unsigned s, i64 nice) {
    volatile i64 *out = shared_page(4096);
    *out = 99;
    i64 pid = fork();
    if (pid == 0) {
        if (syscall2(SYS_setgroups, 0, 0) == 0 && syscall3(SYS_setresuid, r, e, s) == 0)
            *out = setprio(target, nice);
        exit_group(0);
    }
    wait_exit(pid);
    i64 v = *out;
    syscall2(SYS_munmap, (i64)out, 4096);
    return v;
}

static i64 start_target(volatile unsigned *g, unsigned r, unsigned e, unsigned s) {
    i64 pid = fork();
    if (pid == 0) {
        u64 lim[2] = { 0, 0 };
        syscall2(SYS_setrlimit, RLIMIT_NICE, lim); /* lowering then needs privilege */
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresuid, r, e, s);
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(0);
    }
    gate_wait(&g[0]);
    return pid;
}

int test_main(void) {
    volatile unsigned *g = shared_page(4096);
    i64 t = start_target(g, 1111, 2222, 3333);
    i64 r = try_as(t, 1111, 1111, 1111, 2);
    check("euid_is_target_ruid_ok", r == 0, r);
    r = try_as(t, 2222, 2222, 2222, 4);
    check("euid_is_target_euid_ok", r == 0, r);
    r = try_as(t, 4444, 2222, 4444, 5);
    check("only_euid_matches_ok", r == 0, r);
    r = syscall2(SYS_getpriority, PRIO_PROCESS, t);
    check("target_nice_5", r == 15, r);
    r = try_as(t, 2222, 2222, 2222, 3);
    check("owner_lowering_eacces", r == -EACCES, r);
    r = try_as(t, 3333, 3333, 3333, 5);
    check("target_suid_only_eperm", r == -EPERM, r);
    r = try_as(t, 1111, 4444, 4444, 6);
    check("caller_ruid_only_eperm", r == -EPERM, r);
    r = try_as(t, 4444, 4444, 2222, 6);
    check("caller_suid_only_eperm", r == -EPERM, r);
    r = try_as(t, 4444, 4444, 4444, 1);
    check("stranger_lowering_eperm_first", r == -EPERM, r);
    r = try_as(t, 4444, 4444, 4444, 0x3ffffff0);
    check("stranger_any_value_eperm", r == -EPERM, r);
    r = setprio(t, -3);
    check("root_may_lower", r == 0, r);
    r = syscall2(SYS_getpriority, PRIO_PROCESS, t);
    check("target_nice_minus3", r == 23, r);
    gate_open(&g[1]);
    wait_exit(t);

    volatile unsigned *g2 = shared_page(4096);
    t = start_target(g2, 1111, 1111, 0);
    r = try_as(t, 1111, 1111, 1111, 5);
    check("target_with_more_caps_eperm", r == -EPERM, r);
    gate_open(&g2[1]);
    wait_exit(t);
    return finish();
}
